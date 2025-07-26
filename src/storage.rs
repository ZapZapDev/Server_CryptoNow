use std::collections::HashMap;
use tokio::sync::RwLock;
use chrono::{DateTime, Utc};

use crate::payment::Payment;

#[derive(Debug, Clone)]
pub struct StorageService {
    payments: std::sync::Arc<RwLock<HashMap<String, Payment>>>,
}

impl StorageService {
    pub fn new() -> Self {
        Self {
            payments: std::sync::Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Сохранить платеж
    pub async fn save_payment(&self, payment_id: &str, payment: &Payment) -> anyhow::Result<()> {
        let mut payments = self.payments.write().await;
        payments.insert(payment_id.to_string(), payment.clone());

        log::debug!("Payment {} saved to storage", payment_id);
        Ok(())
    }

    /// Получить платеж
    pub async fn get_payment(&self, payment_id: &str) -> anyhow::Result<Option<Payment>> {
        let payments = self.payments.read().await;
        Ok(payments.get(payment_id).cloned())
    }

    /// Удалить платеж
    pub async fn delete_payment(&self, payment_id: &str) -> anyhow::Result<bool> {
        let mut payments = self.payments.write().await;
        Ok(payments.remove(payment_id).is_some())
    }

    /// Получить все платежи (для отладки)
    pub async fn get_all_payments(&self) -> anyhow::Result<HashMap<String, Payment>> {
        let payments = self.payments.read().await;
        Ok(payments.clone())
    }

    /// Очистить просроченные платежи
    pub async fn cleanup_expired_payments(&self) -> anyhow::Result<usize> {
        let mut payments = self.payments.write().await;
        let now = Utc::now();

        let expired_keys: Vec<String> = payments
            .iter()
            .filter(|(_, payment)| now > payment.expires_at)
            .map(|(key, _)| key.clone())
            .collect();

        let count = expired_keys.len();

        for key in expired_keys {
            payments.remove(&key);
        }

        if count > 0 {
            log::info!("Cleaned up {} expired payments", count);
        }

        Ok(count)
    }

    /// Получить статистику
    pub async fn get_stats(&self) -> anyhow::Result<StorageStats> {
        let payments = self.payments.read().await;
        let now = Utc::now();

        let total = payments.len();
        let pending = payments.values()
            .filter(|p| matches!(p.status, crate::payment::PaymentStatus::Pending) && now <= p.expires_at)
            .count();
        let completed = payments.values()
            .filter(|p| matches!(p.status, crate::payment::PaymentStatus::Completed))
            .count();
        let expired = payments.values()
            .filter(|p| now > p.expires_at)
            .count();

        Ok(StorageStats {
            total,
            pending,
            completed,
            expired,
        })
    }
}

#[derive(Debug, serde::Serialize)]
pub struct StorageStats {
    pub total: usize,
    pub pending: usize,
    pub completed: usize,
    pub expired: usize,
}