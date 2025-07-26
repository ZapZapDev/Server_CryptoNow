use serde::{Deserialize, Serialize};
use std::env;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub solana: SolanaConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolanaConfig {
    pub rpc_url: String,
    pub commitment: String,
    pub fee_wallet: String,
    pub fee_amount: f64,
    pub fee_token: String,
    pub supported_tokens: Vec<TokenConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenConfig {
    pub symbol: String,
    pub mint: Option<String>, // None для SOL (native)
    pub decimals: u8,
    pub name: String,
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        // Загружаем из переменных окружения или используем дефолты
        let config = Config {
            server: ServerConfig {
                host: env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string()),
                port: env::var("PORT")
                    .unwrap_or_else(|_| "3001".to_string())
                    .parse()
                    .unwrap_or(3001),
            },
            solana: SolanaConfig {
                rpc_url: env::var("SOLANA_RPC")
                    .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string()),
                commitment: "confirmed".to_string(),

                // ⚠️ ПОМЕНЯЙ НА СВОЙ КОШЕЛЕК ДЛЯ КОМИССИЙ!
                fee_wallet: env::var("FEE_WALLET")
                    .unwrap_or_else(|_| "9E9ME8Xjrnnz5tyLqPWUbXVbPjXusEp9NdjKeugDjW5t".to_string()),

                fee_amount: env::var("FEE_AMOUNT")
                    .unwrap_or_else(|_| "1.0".to_string())
                    .parse()
                    .unwrap_or(1.0),

                fee_token: env::var("FEE_TOKEN")
                    .unwrap_or_else(|_| "USDC".to_string()),

                supported_tokens: vec![
                    TokenConfig {
                        symbol: "SOL".to_string(),
                        mint: None, // Native SOL
                        decimals: 9,
                        name: "Solana".to_string(),
                    },
                    TokenConfig {
                        symbol: "USDC".to_string(),
                        mint: Some("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string()),
                        decimals: 6,
                        name: "USD Coin".to_string(),
                    },
                    TokenConfig {
                        symbol: "USDT".to_string(),
                        mint: Some("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB".to_string()),
                        decimals: 6,
                        name: "Tether USD".to_string(),
                    },
                ],
            },
        };

        // Валидация конфигурации
        config.validate()?;

        Ok(config)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        // Проверяем что кошелек для комиссий настроен
        if self.solana.fee_wallet == "YOUR_SOLANA_WALLET_HERE" {
            anyhow::bail!("⚠️  Please set your fee wallet in FEE_WALLET environment variable or config!");
        }

        // Проверяем что сумма комиссии положительная
        if self.solana.fee_amount <= 0.0 {
            anyhow::bail!("Fee amount must be positive");
        }

        // Проверяем что есть поддерживаемые токены
        if self.solana.supported_tokens.is_empty() {
            anyhow::bail!("No supported tokens configured");
        }

        Ok(())
    }

    pub fn get_token_config(&self, symbol: &str) -> Option<&TokenConfig> {
        self.solana.supported_tokens.iter().find(|t| t.symbol == symbol)
    }

    pub fn is_token_supported(&self, symbol: &str) -> bool {
        self.get_token_config(symbol).is_some()
    }

    pub fn get_supported_tokens(&self) -> Vec<String> {
        self.solana.supported_tokens.iter()
            .map(|t| t.symbol.clone())
            .collect()
    }
}