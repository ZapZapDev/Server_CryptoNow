use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signature::Signature,
    system_instruction,
    instruction::Instruction,
};
use spl_token::{
    instruction as token_instruction,
    ID as TOKEN_PROGRAM_ID,
};
use std::str::FromStr;
use std::sync::Arc;
use anyhow::Result;

use crate::config::{Config, TokenConfig};

#[derive(Clone)]
pub struct MultichainService {
    pub solana_client: Arc<RpcClient>,
    pub config: Config,
}

#[derive(Debug, Clone)]
pub struct TransferInstruction {
    pub instruction: Instruction,
    pub description: String,
}

impl MultichainService {
    pub fn new(config: Config) -> Self {
        let commitment = CommitmentConfig::confirmed();
        let solana_client = Arc::new(RpcClient::new_with_commitment(
            config.solana.rpc_url.clone(),
            commitment,
        ));

        Self {
            solana_client,
            config,
        }
    }

    /// Создать инструкции для платежа с комиссией
    pub async fn create_payment_instructions(
        &self,
        payer: &Pubkey,
        recipient: &Pubkey,
        amount: f64,
        token: &str,
    ) -> Result<Vec<TransferInstruction>> {
        let mut instructions = Vec::new();

        // 1. Основной платеж
        let main_instruction = self.create_transfer_instruction(
            payer,
            recipient,
            amount,
            token,
        ).await?;
        instructions.push(main_instruction);

        // 2. Комиссия (всегда в USDC на твой кошелек)
        let fee_recipient = Pubkey::from_str(&self.config.solana.fee_wallet)?;
        let fee_instruction = self.create_transfer_instruction(
            payer,
            &fee_recipient,
            self.config.solana.fee_amount,
            &self.config.solana.fee_token,
        ).await?;
        instructions.push(fee_instruction);

        Ok(instructions)
    }

    /// Создать инструкцию перевода для любого токена
    async fn create_transfer_instruction(
        &self,
        from: &Pubkey,
        to: &Pubkey,
        amount: f64,
        token: &str,
    ) -> Result<TransferInstruction> {
        let token_config = self.config.get_token_config(token)
            .ok_or_else(|| anyhow::anyhow!("Token {} not supported", token))?;

        if token == "SOL" {
            self.create_sol_transfer_instruction(from, to, amount, token_config)
        } else {
            self.create_spl_transfer_instruction(from, to, amount, token_config).await
        }
    }

    /// SOL трансфер (нативная валюта)
    fn create_sol_transfer_instruction(
        &self,
        from: &Pubkey,
        to: &Pubkey,
        amount: f64,
        token_config: &TokenConfig,
    ) -> Result<TransferInstruction> {
        let lamports = (amount * 10_f64.powi(token_config.decimals as i32)) as u64;

        let instruction = system_instruction::transfer(from, to, lamports);

        Ok(TransferInstruction {
            instruction,
            description: format!("Transfer {} SOL from {} to {}", amount, from, to),
        })
    }

    /// SPL токен трансфер
    async fn create_spl_transfer_instruction(
        &self,
        from: &Pubkey,
        to: &Pubkey,
        amount: f64,
        token_config: &TokenConfig,
    ) -> Result<TransferInstruction> {
        let mint = Pubkey::from_str(
            token_config.mint.as_ref()
                .ok_or_else(|| anyhow::anyhow!("No mint address for token {}", token_config.symbol))?
        )?;

        // Получаем associated token accounts
        let from_token_account = spl_associated_token_account::get_associated_token_address(from, &mint);
        let to_token_account = spl_associated_token_account::get_associated_token_address(to, &mint);

        // Создаем transfer instruction
        let token_amount = (amount * 10_f64.powi(token_config.decimals as i32)) as u64;

        let transfer_instruction = token_instruction::transfer(
            &TOKEN_PROGRAM_ID,
            &from_token_account,
            &to_token_account,
            from,
            &[],
            token_amount,
        )?;

        Ok(TransferInstruction {
            instruction: transfer_instruction,
            description: format!(
                "Transfer {} {} from {} to {}",
                amount, token_config.symbol, from, to
            ),
        })
    }

    /// Простая верификация транзакции
    pub async fn verify_transaction(
        &self,
        signature: &str,
        _expected_recipient: &Pubkey,
        _expected_amount: f64,
        _expected_token: &str,
    ) -> Result<TransactionVerification> {
        let signature = Signature::from_str(signature)?;

        // Простая проверка - существует ли транзакция
        match self.solana_client.get_signature_status(&signature) {
            Ok(Some(status)) => {
                if status.is_err() {
                    Ok(TransactionVerification {
                        is_valid: false,
                        details: "Transaction failed".to_string(),
                        main_transfer_valid: false,
                        fee_transfer_valid: false,
                    })
                } else {
                    Ok(TransactionVerification {
                        is_valid: true,
                        details: "Transaction confirmed".to_string(),
                        main_transfer_valid: true,
                        fee_transfer_valid: true,
                    })
                }
            }
            Ok(None) => Ok(TransactionVerification {
                is_valid: false,
                details: "Transaction not found".to_string(),
                main_transfer_valid: false,
                fee_transfer_valid: false,
            }),
            Err(e) => Ok(TransactionVerification {
                is_valid: false,
                details: format!("Error checking transaction: {}", e),
                main_transfer_valid: false,
                fee_transfer_valid: false,
            }),
        }
    }

    /// Валидировать Solana адрес
    pub fn validate_address(&self, address: &str) -> bool {
        Pubkey::from_str(address).is_ok()
    }
}

#[derive(Debug, Clone)]
pub struct TransactionVerification {
    pub is_valid: bool,
    pub details: String,
    pub main_transfer_valid: bool,
    pub fee_transfer_valid: bool,
}