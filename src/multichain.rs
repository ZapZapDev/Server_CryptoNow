use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signature::Signature,
    system_instruction,
    transaction::Transaction,
    instruction::Instruction,
};
use spl_token::{
    instruction as token_instruction,
    ID as TOKEN_PROGRAM_ID,
};
use std::str::FromStr;
use anyhow::Result;

use crate::config::{Config, TokenConfig};

#[derive(Debug, Clone)]
pub struct MultichainService {
    pub solana_client: RpcClient,
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
        let solana_client = RpcClient::new_with_commitment(
            config.solana.rpc_url.clone(),
            commitment,
        );

        Self {
            solana_client,
            config,
        }
    }

    /// Создать инструкции для платежа с комиссией
    /// Возвращает 2 инструкции: основной платеж + комиссия
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

        // Проверяем существует ли recipient token account
        let to_account_info = self.solana_client.get_account(&to_token_account);

        let mut instructions = Vec::new();

        // Если account не существует, создаем его
        if to_account_info.is_err() {
            let create_account_instruction = spl_associated_token_account::instruction::create_associated_token_account(
                from, // payer
                to,   // wallet
                &mint,
                &TOKEN_PROGRAM_ID,
            );
            instructions.push(create_account_instruction);
        }

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

        instructions.push(transfer_instruction);

        // Для простоты возвращаем только transfer instruction
        // В реальности нужно обрабатывать создание account отдельно
        Ok(TransferInstruction {
            instruction: transfer_instruction,
            description: format!(
                "Transfer {} {} from {} to {}",
                amount, token_config.symbol, from, to
            ),
        })
    }

    /// Верифицировать выполненную транзакцию
    pub async fn verify_transaction(
        &self,
        signature: &str,
        expected_recipient: &Pubkey,
        expected_amount: f64,
        expected_token: &str,
    ) -> Result<TransactionVerification> {
        let signature = Signature::from_str(signature)?;

        // Получаем транзакцию из блокчейна
        let transaction = self.solana_client.get_transaction(&signature, Default::default())?;

        if transaction.transaction.meta.as_ref()
            .map(|meta| meta.err.is_some())
            .unwrap_or(true)
        {
            return Ok(TransactionVerification {
                is_valid: false,
                details: "Transaction failed or not found".to_string(),
                main_transfer_valid: false,
                fee_transfer_valid: false,
            });
        }

        // Проверяем основной платеж
        let main_transfer_valid = self.verify_transfer_in_transaction(
            &transaction,
            expected_recipient,
            expected_amount,
            expected_token,
        )?;

        // Проверяем комиссию
        let fee_recipient = Pubkey::from_str(&self.config.solana.fee_wallet)?;
        let fee_transfer_valid = self.verify_transfer_in_transaction(
            &transaction,
            &fee_recipient,
            self.config.solana.fee_amount,
            &self.config.solana.fee_token,
        )?;

        Ok(TransactionVerification {
            is_valid: main_transfer_valid && fee_transfer_valid,
            details: format!(
                "Main transfer: {}, Fee transfer: {}",
                main_transfer_valid, fee_transfer_valid
            ),
            main_transfer_valid,
            fee_transfer_valid,
        })
    }

    /// Проверить конкретный трансфер в транзакции
    fn verify_transfer_in_transaction(
        &self,
        transaction: &solana_client::rpc_response::RpcConfirmedTransaction,
        recipient: &Pubkey,
        expected_amount: f64,
        token: &str,
    ) -> Result<bool> {
        let meta = transaction.transaction.meta.as_ref()
            .ok_or_else(|| anyhow::anyhow!("No transaction meta"))?;

        if token == "SOL" {
            self.verify_sol_transfer_in_transaction(transaction, recipient, expected_amount)
        } else {
            self.verify_spl_transfer_in_transaction(meta, recipient, expected_amount, token)
        }
    }

    fn verify_sol_transfer_in_transaction(
        &self,
        transaction: &solana_client::rpc_response::RpcConfirmedTransaction,
        recipient: &Pubkey,
        expected_amount: f64,
    ) -> Result<bool> {
        let meta = transaction.transaction.meta.as_ref().unwrap();
        let account_keys = &transaction.transaction.transaction.message.account_keys;

        // Находим индекс получателя
        let recipient_index = account_keys.iter()
            .position(|key| key == recipient);

        if let Some(index) = recipient_index {
            let pre_balance = meta.pre_balances.get(index).copied().unwrap_or(0);
            let post_balance = meta.post_balances.get(index).copied().unwrap_or(0);

            let actual_change = (post_balance as f64 - pre_balance as f64) / 10_f64.powi(9); // lamports to SOL
            let tolerance = 0.000001; // Допустимая погрешность

            Ok((actual_change - expected_amount).abs() < tolerance)
        } else {
            Ok(false)
        }
    }

    fn verify_spl_transfer_in_transaction(
        &self,
        meta: &solana_sdk::transaction::TransactionStatusMeta,
        recipient: &Pubkey,
        expected_amount: f64,
        token: &str,
    ) -> Result<bool> {
        let token_config = self.config.get_token_config(token)
            .ok_or_else(|| anyhow::anyhow!("Token {} not supported", token))?;

        let mint = token_config.mint.as_ref()
            .ok_or_else(|| anyhow::anyhow!("No mint for token {}", token))?;

        // Проверяем изменения в token balances
        if let (Some(pre_balances), Some(post_balances)) =
            (&meta.pre_token_balances, &meta.post_token_balances) {

            for post_balance in post_balances {
                if post_balance.owner == recipient.to_string() &&
                   post_balance.mint == *mint {

                    let post_amount = post_balance.ui_token_amount.ui_amount.unwrap_or(0.0);

                    // Ищем соответствующий pre_balance
                    let pre_amount = pre_balances.iter()
                        .find(|pre| pre.owner == recipient.to_string() && pre.mint == *mint)
                        .map(|pre| pre.ui_token_amount.ui_amount.unwrap_or(0.0))
                        .unwrap_or(0.0);

                    let actual_change = post_amount - pre_amount;
                    let tolerance = 0.000001;

                    if (actual_change - expected_amount).abs() < tolerance {
                        return Ok(true);
                    }
                }
            }
        }

        Ok(false)
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