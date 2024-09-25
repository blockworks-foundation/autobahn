use serde_derive::{Deserialize, Serialize};
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
#[serde_with::serde_as]
pub struct SwapResponse {
    #[serde_as(as = "Base64")]
    pub swap_transaction: Vec<u8>,
    pub last_valid_block_height: u64,
    pub priorization_fee_lamports: u64,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SwapIxResponse {
    pub token_ledger_instruction: Option<InstructionResponse>,
    pub compute_budget_instructions: Option<Vec<InstructionResponse>>,
    pub setup_instructions: Option<Vec<InstructionResponse>>,
    pub swap_instruction: InstructionResponse,
    pub cleanup_instructions: Option<Vec<InstructionResponse>>,
    pub address_lookup_table_addresses: Option<Vec<String>>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct InstructionResponse {
    pub program_id: String,
    pub data: Option<String>,
    pub accounts: Option<Vec<AccountMeta>>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AccountMeta {
    pub pubkey: String,
    pub is_signer: Option<bool>,
    pub is_writable: Option<bool>,
}

impl InstructionResponse {
    pub fn from_ix(instruction: Instruction) -> anyhow::Result<InstructionResponse> {
        Ok(Self {
            program_id: instruction.program_id.to_string(),
            data: Some(base64::encode(instruction.data)),
            accounts: Some(
                instruction
                    .accounts
                    .into_iter()
                    .map(|x| AccountMeta {
                        pubkey: x.pubkey.to_string(),
                        is_signer: Some(x.is_signer),
                        is_writable: Some(x.is_writable),
                    })
                    .collect(),
            ),
        })
    }

    pub fn to_ix(&self) -> anyhow::Result<Instruction> {
        self.try_into()
    }
}

impl TryFrom<&InstructionResponse> for solana_sdk::instruction::Instruction {
    type Error = anyhow::Error;
    fn try_from(m: &InstructionResponse) -> Result<Self, Self::Error> {
        Ok(Self {
            program_id: Pubkey::from_str(&m.program_id)?,
            data: m.data.as_ref().map(base64::decode).unwrap_or(Ok(vec![]))?,
            accounts: m
                .accounts
                .as_ref()
                .map(|accs| {
                    accs.iter()
                        .map(|a| a.try_into())
                        .collect::<anyhow::Result<Vec<solana_sdk::instruction::AccountMeta>>>()
                })
                .unwrap_or(Ok(vec![]))?,
        })
    }
}

impl TryFrom<&AccountMeta> for solana_sdk::instruction::AccountMeta {
    type Error = anyhow::Error;
    fn try_from(m: &AccountMeta) -> Result<Self, Self::Error> {
        Ok(Self {
            pubkey: Pubkey::from_str(&m.pubkey)?,
            is_signer: m.is_signer.unwrap_or(false),
            is_writable: m.is_writable.unwrap_or(false),
        })
    }
}
