use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use uuid::Uuid;
use chrono::{DateTime, Utc, Duration};

use crate::config::Config;
use crate::multichain::MultichainService;
use crate::qr::QrService;
use crate::storage::StorageService;

#[derive(Clone)]
pub struct PaymentService {
    multichain: MultichainService,
    qr_service: QrService,
    storage: StorageService,
    config: Config,
}

#[derive(Debug, Deserialize)]
pub struct CreatePaymentRequest {
    pub recipient: String,
    pub amount: f64,
    pub token: String,
    pub label: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct Payment {
    pub id: String,
    pub recipient: String,
    pub amount: f64,
    pub token: String,
    pub fee_recipient: String,
    pub fee_amount: f64,
    pub fee_token: String,
    pub label: String,
    pub message: String,
    pub url: String,
    pub qr_code: String,
    pub status: PaymentStatus,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub signature: Option<String>,
    pub verified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum PaymentStatus {
    Pending,
    Completed,
    Expired,
    Failed,
}

#[derive(Debug, Serialize)]
pub struct PaymentResponse {
    pub success: bool,
    pub data: Option<Payment>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct VerificationResult {
    pub success: bool,
    pub status: PaymentStatus,
    pub verified: bool,
    pub signature: Option<String>,
    pub details: String,
}

impl PaymentService {
    pub async fn new(config: Config) -> anyhow::Result<Self> {
        let multichain = MultichainService::new(config.clone());
        let qr_service = QrService::new();
        let storage = StorageService::new();

        Ok(Self {
            multichain,
            qr_service,
            storage,
            config,
        })
    }

    /// Создать платеж с автоматической комиссией
    pub async fn create_payment_with_fee(
        &self,
        request: CreatePaymentRequest,
    ) -> anyhow::Result<Payment> {
        // Валидация входных данных
        self.validate_payment_request(&request)?;

        // Генерируем уникальный ID
        let payment_id = format!("pay_{}", Uuid::new_v4().simple());

        // Создаем Solana Pay URL с комиссией
        let (url, qr_code) = self.create_solana_pay_url(&request, &payment_id).await?;

        // Создаем объект платежа
        let now = Utc::now();
        let payment = Payment {
            id: payment_id.clone(),
            recipient: request.recipient.clone(),
            amount: request.amount,
            token: request.token.clone(),
            fee_recipient: self.config.solana.fee_wallet.clone(),
            fee_amount: self.config.solana.fee_amount,
            fee_token: self.config.solana.fee_token.clone(),
            label: request.label.unwrap_or_else(|| format!("Payment {}", request.token)),
            message: request.message.unwrap_or_else(|| {
                format!("{} {} + {} {} fee",
                        request.amount, request.token,
                        self.config.solana.fee_amount, self.config.solana.fee_token)
            }),
            url,
            qr_code,
            status: PaymentStatus::Pending,
            created_at: now,
            expires_at: now + Duration::minutes(30),
            signature: None,
            verified_at: None,
        };

        // Сохраняем в storage
        self.storage.save_payment(&payment_id, &payment).await?;

        log::info!("Payment created: {} for {} {} + {} {} fee",
            payment_id, request.amount, request.token,
            self.config.solana.fee_amount, self.config.solana.fee_token);

        Ok(payment)
    }

    /// Создать Solana Pay URL с комиссией
    async fn create_solana_pay_url(
        &self,
        request: &CreatePaymentRequest,
        payment_id: &str,
    ) -> anyhow::Result<(String, String)> {
        // Твой ngrok URL
        let ngrok_url = "https://e266cfdadf7e.ngrok-free.app";

        // Создаем правильный Solana Pay Transaction Request URL
        let transaction_request_url = format!(
            "solana:{}/api/payment/{}/transaction",
            ngrok_url, payment_id
        );

        // Генерируем QR код
        let qr_code = self.qr_service.generate_qr_code(&transaction_request_url)?;

        log::info!("Generated QR URL: {}", transaction_request_url);

        Ok((transaction_request_url, qr_code))
    }

    /// Получить информацию о платеже
    pub async fn get_payment(&self, payment_id: &str) -> anyhow::Result<Option<Payment>> {
        self.storage.get_payment(payment_id).await
    }

    /// Верифицировать платеж по подписи транзакции
    pub async fn verify_payment(
        &self,
        payment_id: &str,
        signature: &str,
    ) -> anyhow::Result<VerificationResult> {
        // Получаем платеж
        let mut payment = self.storage.get_payment(payment_id).await?
            .ok_or_else(|| anyhow::anyhow!("Payment not found"))?;

        // Проверяем не истек ли платеж
        if Utc::now() > payment.expires_at {
            payment.status = PaymentStatus::Expired;
            self.storage.save_payment(payment_id, &payment).await?;

            return Ok(VerificationResult {
                success: false,
                status: PaymentStatus::Expired,
                verified: false,
                signature: None,
                details: "Payment has expired".to_string(),
            });
        }

        // Если уже верифицирован
        if matches!(payment.status, PaymentStatus::Completed) {
            return Ok(VerificationResult {
                success: true,
                status: PaymentStatus::Completed,
                verified: true,
                signature: payment.signature.clone(),
                details: "Already verified".to_string(),
            });
        }

        // Верифицируем в блокчейне
        let recipient = Pubkey::from_str(&payment.recipient)?;
        let verification = self.multichain.verify_transaction(
            signature,
            &recipient,
            payment.amount,
            &payment.token,
        ).await?;

        if verification.is_valid {
            // Обновляем статус платежа
            payment.status = PaymentStatus::Completed;
            payment.signature = Some(signature.to_string());
            payment.verified_at = Some(Utc::now());

            self.storage.save_payment(payment_id, &payment).await?;

            log::info!("Payment {} verified successfully with signature {}",
                payment_id, signature);

            Ok(VerificationResult {
                success: true,
                status: PaymentStatus::Completed,
                verified: true,
                signature: Some(signature.to_string()),
                details: verification.details,
            })
        } else {
            log::warn!("Payment {} verification failed: {}",
                payment_id, verification.details);

            Ok(VerificationResult {
                success: false,
                status: PaymentStatus::Pending,
                verified: false,
                signature: None,
                details: verification.details,
            })
        }
    }

    /// Валидация запроса на создание платежа
    fn validate_payment_request(&self, request: &CreatePaymentRequest) -> anyhow::Result<()> {
        // Проверяем адрес получателя
        if !self.multichain.validate_address(&request.recipient) {
            anyhow::bail!("Invalid recipient address: {}", request.recipient);
        }

        // Проверяем сумму
        if request.amount <= 0.0 {
            anyhow::bail!("Amount must be positive, got: {}", request.amount);
        }

        // Проверяем поддерживается ли токен
        if !self.config.is_token_supported(&request.token) {
            let supported = self.config.get_supported_tokens();
            anyhow::bail!(
                "Token {} not supported. Supported tokens: {}",
                request.token,
                supported.join(", ")
            );
        }

        // Проверяем разумные лимиты
        if request.amount > 1_000_000.0 {
            anyhow::bail!("Amount too large: {}", request.amount);
        }

        Ok(())
    }

    /// Очистка просроченных платежей
    pub async fn cleanup_expired_payments(&self) -> anyhow::Result<usize> {
        self.storage.cleanup_expired_payments().await
    }
}