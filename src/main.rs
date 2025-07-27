use actix_cors::Cors;
use actix_web::{web, App, HttpResponse, HttpServer, Result, middleware::Logger};
use serde::{Deserialize, Serialize};
use solana_sdk::{
    transaction::Transaction,
    pubkey::Pubkey,
    system_instruction,
};
use spl_token::instruction as token_instruction;
use std::str::FromStr;
use base64::{Engine as _, engine::general_purpose};
use tokio::time::{timeout, Duration};

mod config;
mod multichain;
mod payment;
mod qr;
mod storage;

use config::Config;
use payment::{PaymentService, CreatePaymentRequest, PaymentResponse};

#[derive(Serialize)]
struct ServerInfo {
    message: String,
    status: String,
    version: String,
    supported_networks: Vec<String>,
}

#[derive(Deserialize)]
struct TransactionRequestPost {
    account: String,
}

#[derive(Serialize)]
struct TransactionRequestGet {
    label: String,
    icon: String,
}

#[derive(Serialize)]
struct TransactionResponse {
    transaction: String,
    message: Option<String>,
}

// Главная страница API
async fn index() -> Result<HttpResponse> {
    let info = ServerInfo {
        message: "CryptoNow Rust API Server 🦀".to_string(),
        status: "running".to_string(),
        version: "1.0.0".to_string(),
        supported_networks: vec!["solana".to_string()],
    };
    Ok(HttpResponse::Ok().json(info))
}

// Создать платеж с комиссией
async fn create_payment(
    payment_service: web::Data<PaymentService>,
    req: web::Json<CreatePaymentRequest>,
) -> Result<HttpResponse> {
    log::info!("Creating payment: {:?}", req);

    match payment_service.create_payment_with_fee(req.into_inner()).await {
        Ok(payment) => {
            log::info!("Payment created successfully: {}", payment.id);
            Ok(HttpResponse::Ok().json(PaymentResponse {
                success: true,
                data: Some(payment),
                error: None,
            }))
        }
        Err(e) => {
            log::error!("Payment creation failed: {}", e);
            Ok(HttpResponse::BadRequest().json(PaymentResponse {
                success: false,
                data: None,
                error: Some(e.to_string()),
            }))
        }
    }
}

// GET: Метаданные для Solana Pay
async fn transaction_get(
    payment_service: web::Data<PaymentService>,
    path: web::Path<String>,
) -> Result<HttpResponse> {
    let payment_id = path.into_inner();
    log::info!("GET transaction metadata for payment: {}", payment_id);

    match payment_service.get_payment(&payment_id).await {
        Ok(Some(payment)) => {
            Ok(HttpResponse::Ok()
                .append_header(("Content-Type", "application/json"))
                .append_header(("Access-Control-Allow-Origin", "*"))
                .json(TransactionRequestGet {
                    label: format!("Pay {} {} + {} {} fee",
                                   payment.amount, payment.token,
                                   payment.fee_amount, payment.fee_token),
                    icon: "https://solana.com/src/img/branding/solanaLogoMark.svg".to_string(),
                }))
        }
        Ok(None) => Ok(HttpResponse::NotFound().json(serde_json::json!({"error": "Payment not found"}))),
        Err(e) => Ok(HttpResponse::InternalServerError().json(serde_json::json!({"error": e.to_string()})))
    }
}

// POST: Создание транзакции для Solana Pay
async fn transaction_post(
    payment_service: web::Data<PaymentService>,
    path: web::Path<String>,
    req: web::Json<TransactionRequestPost>,
) -> Result<HttpResponse> {
    let payment_id = path.into_inner();
    let account = req.account.clone();

    log::info!("POST transaction request for payment {} from account {}", payment_id, account);

    // Получаем платеж
    let payment = match payment_service.get_payment(&payment_id).await {
        Ok(Some(payment)) => payment,
        Ok(None) => {
            return Ok(HttpResponse::NotFound()
                .append_header(("Content-Type", "application/json"))
                .json(serde_json::json!({"error": "Payment not found"})));
        }
        Err(e) => {
            return Ok(HttpResponse::InternalServerError()
                .append_header(("Content-Type", "application/json"))
                .json(serde_json::json!({"error": e.to_string()})));
        }
    };

    // Создаем транзакцию с таймаутом и fallback
    match timeout(Duration::from_secs(10), create_payment_transaction(&payment, &account)).await {
        Ok(Ok(transaction_base64)) => {
            log::info!("Transaction created successfully for payment {}", payment_id);
            Ok(HttpResponse::Ok()
                .append_header(("Content-Type", "application/json"))
                .append_header(("Access-Control-Allow-Origin", "*"))
                .json(TransactionResponse {
                    transaction: transaction_base64,
                    message: Some(format!("Pay {} {} + {} {} fee",
                                          payment.amount, payment.token,
                                          payment.fee_amount, payment.fee_token)),
                }))
        }
        Ok(Err(e)) => {
            log::error!("Transaction creation failed for payment {}: {}", payment_id, e);
            Ok(HttpResponse::BadRequest()
                .append_header(("Content-Type", "application/json"))
                .json(serde_json::json!({"error": e.to_string()})))
        }
        Err(_) => {
            log::error!("Transaction creation timed out for payment {}", payment_id);
            Ok(HttpResponse::RequestTimeout()
                .append_header(("Content-Type", "application/json"))
                .json(serde_json::json!({"error": "Transaction creation timed out"})))
        }
    }
}

// ОПТИМИЗИРОВАННАЯ функция создания транзакции
async fn create_payment_transaction(
    payment: &payment::Payment,
    payer_str: &str,
) -> anyhow::Result<String> {
    let payer = Pubkey::from_str(payer_str)?;
    let recipient = Pubkey::from_str(&payment.recipient)?;
    let fee_recipient = Pubkey::from_str(&payment.fee_recipient)?;

    let mut instructions = Vec::new();

    // 1. Основной платеж
    if payment.token == "SOL" {
        let lamports = (payment.amount * 1_000_000_000.0) as u64;
        instructions.push(system_instruction::transfer(&payer, &recipient, lamports));
    } else {
        let mint = match payment.token.as_str() {
            "USDC" => Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")?,
            "USDT" => Pubkey::from_str("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB")?,
            _ => anyhow::bail!("Unsupported token: {}", payment.token),
        };

        let decimals = if payment.token == "USDC" || payment.token == "USDT" { 6 } else { 9 };
        let amount = (payment.amount * 10_f64.powi(decimals)) as u64;

        let from_token_account = spl_associated_token_account::get_associated_token_address(&payer, &mint);
        let to_token_account = spl_associated_token_account::get_associated_token_address(&recipient, &mint);

        // Создание ATA для получателя
        instructions.push(
            spl_associated_token_account::instruction::create_associated_token_account(
                &payer, &recipient, &mint, &spl_token::ID,
            )
        );

        // Основной transfer
        instructions.push(token_instruction::transfer(
            &spl_token::ID, &from_token_account, &to_token_account, &payer, &[], amount,
        )?);
    }

    // 2. Комиссия 1 USDC
    let usdc_mint = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v")?;
    let fee_amount = (payment.fee_amount * 1_000_000.0) as u64;

    let from_usdc_account = spl_associated_token_account::get_associated_token_address(&payer, &usdc_mint);
    let to_usdc_account = spl_associated_token_account::get_associated_token_address(&fee_recipient, &usdc_mint);

    // Создание ATA для fee кошелька
    instructions.push(
        spl_associated_token_account::instruction::create_associated_token_account(
            &payer, &fee_recipient, &usdc_mint, &spl_token::ID,
        )
    );

    // Fee transfer
    instructions.push(token_instruction::transfer(
        &spl_token::ID, &from_usdc_account, &to_usdc_account, &payer, &[], fee_amount,
    )?);

    // БЫСТРОЕ получение blockhash с множественными RPC
    let recent_blockhash = get_recent_blockhash_fast().await?;

    // Создание транзакции
    let mut transaction = Transaction::new_with_payer(&instructions, Some(&payer));
    transaction.message.recent_blockhash = recent_blockhash;

    // Сериализация
    let serialized = bincode::serialize(&transaction)?;
    let base64_transaction = general_purpose::STANDARD.encode(serialized);

    log::info!("Transaction with {} instructions created for payment {}",
        instructions.len(), payment.id);

    Ok(base64_transaction)
}

// БЫСТРАЯ функция получения blockhash с fallback RPC
async fn get_recent_blockhash_fast() -> anyhow::Result<solana_sdk::hash::Hash> {
    use solana_client::rpc_client::RpcClient;
    use solana_sdk::commitment_config::CommitmentConfig;

    // Список RPC endpoints для fallback
    let rpc_endpoints = [
        "https://api.mainnet-beta.solana.com",
        "https://solana-api.projectserum.com",
        "https://rpc.ankr.com/solana",
    ];

    for endpoint in &rpc_endpoints {
        match timeout(Duration::from_secs(3), async {
            let client = RpcClient::new_with_commitment(endpoint.to_string(), CommitmentConfig::confirmed());
            client.get_latest_blockhash()
        }).await {
            Ok(Ok(blockhash)) => {
                log::info!("Got blockhash from {}", endpoint);
                return Ok(blockhash);
            }
            Ok(Err(e)) => log::warn!("RPC {} failed: {}", endpoint, e),
            Err(_) => log::warn!("RPC {} timed out", endpoint),
        }
    }

    anyhow::bail!("All RPC endpoints failed or timed out")
}

// Остальные функции (get_payment, verify_payment, main) остаются без изменений
async fn get_payment(
    payment_service: web::Data<PaymentService>,
    path: web::Path<String>,
) -> Result<HttpResponse> {
    let payment_id = path.into_inner();
    match payment_service.get_payment(&payment_id).await {
        Ok(Some(payment)) => Ok(HttpResponse::Ok().json(PaymentResponse {
            success: true, data: Some(payment), error: None,
        })),
        Ok(None) => Ok(HttpResponse::NotFound().json(PaymentResponse {
            success: false, data: None, error: Some("Payment not found".to_string()),
        })),
        Err(e) => Ok(HttpResponse::InternalServerError().json(PaymentResponse {
            success: false, data: None, error: Some(e.to_string()),
        })),
    }
}

#[derive(Deserialize)]
struct VerifyPaymentRequest {
    signature: String,
}

async fn verify_payment(
    payment_service: web::Data<PaymentService>,
    path: web::Path<String>,
    req: web::Json<VerifyPaymentRequest>,
) -> Result<HttpResponse> {
    let payment_id = path.into_inner();
    let signature = req.signature.clone();
    match payment_service.verify_payment(&payment_id, &signature).await {
        Ok(verification) => Ok(HttpResponse::Ok().json(verification)),
        Err(e) => Ok(HttpResponse::BadRequest().json(serde_json::json!({
           "success": false, "error": e.to_string()
       }))),
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init();
    println!("🦀 Starting CryptoNow Rust Server...");

    let config = Config::load().expect("Failed to load config");
    let payment_service = PaymentService::new(config.clone()).await.expect("Failed to initialize payment service");

    let host = config.server.host.clone();
    let port = config.server.port;

    println!("🚀 Server starting on http://{}:{}", host, port);
    println!("📡 Fee wallet: {}", config.solana.fee_wallet);
    println!("💰 Fee amount: {} {}", config.solana.fee_amount, config.solana.fee_token);

    HttpServer::new(move || {
        let cors = Cors::default().allow_any_origin().allow_any_method().allow_any_header().max_age(3600);
        App::new()
            .app_data(web::Data::new(payment_service.clone()))
            .wrap(cors)
            .wrap(Logger::default())
            .route("/", web::get().to(index))
            .service(
                web::scope("/api")
                    .route("/payment/create", web::post().to(create_payment))
                    .route("/payment/{id}", web::get().to(get_payment))
                    .route("/payment/{id}/transaction", web::get().to(transaction_get))
                    .route("/payment/{id}/transaction", web::post().to(transaction_post))
                    .route("/payment/{id}/verify", web::post().to(verify_payment))
            )
    })
        .bind(format!("{}:{}", host, port))?
        .run()
        .await
}