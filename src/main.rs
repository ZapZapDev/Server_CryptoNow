use actix_cors::Cors;
use actix_web::{web, App, HttpResponse, HttpServer, Result, middleware::Logger};
use serde::{Deserialize, Serialize};

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

// Получить информацию о платеже
async fn get_payment(
    payment_service: web::Data<PaymentService>,
    path: web::Path<String>,
) -> Result<HttpResponse> {
    let payment_id = path.into_inner();
    
    match payment_service.get_payment(&payment_id).await {
        Ok(Some(payment)) => Ok(HttpResponse::Ok().json(PaymentResponse {
            success: true,
            data: Some(payment),
            error: None,
        })),
        Ok(None) => Ok(HttpResponse::NotFound().json(PaymentResponse {
            success: false,
            data: None,
            error: Some("Payment not found".to_string()),
        })),
        Err(e) => Ok(HttpResponse::InternalServerError().json(PaymentResponse {
            success: false,
            data: None,
            error: Some(e.to_string()),
        })),
    }
}

// Верифицировать платеж
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
            "success": false,
            "error": e.to_string()
        }))),
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Инициализация логирования
    env_logger::init();
    
    println!("🦀 Starting CryptoNow Rust Server...");
    
    // Загрузка конфигурации
    let config = Config::load().expect("Failed to load config");
    println!("✅ Configuration loaded");
    
    // Инициализация сервисов
    let payment_service = PaymentService::new(config.clone()).await
        .expect("Failed to initialize payment service");
    println!("✅ Payment service initialized");
    
    let host = config.server.host.clone();
    let port = config.server.port;
    
    println!("🚀 Server starting on http://{}:{}", host, port);
    println!("📡 Fee wallet: {}", config.solana.fee_wallet);
    println!("💰 Fee amount: {} {}", config.solana.fee_amount, config.solana.fee_token);
    
    // Запуск HTTP сервера
    HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header()
            .max_age(3600);
            
        App::new()
            .app_data(web::Data::new(payment_service.clone()))
            .wrap(cors)
            .wrap(Logger::default())
            .route("/", web::get().to(index))
            .service(
                web::scope("/api")
                    .route("/payment/create", web::post().to(create_payment))
                    .route("/payment/{id}", web::get().to(get_payment))
                    .route("/payment/{id}/verify", web::post().to(verify_payment))
            )
    })
    .bind(format!("{}:{}", host, port))?
    .run()
    .await
}