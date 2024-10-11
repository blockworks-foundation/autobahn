use anchor_client::ClientError;
use anchor_lang::Discriminator;
use anyhow::{Error, Result};
use colorful::Color;
use colorful::Colorful;
use hex;
use raydium_cp_swap::instruction;
use raydium_cp_swap::states::*;
use regex::Regex;
use solana_transaction_status::{
    option_serializer::OptionSerializer, EncodedTransaction, UiTransactionStatusMeta,
};

const PROGRAM_LOG: &str = "Program log: ";
const PROGRAM_DATA: &str = "Program data: ";
use anchor_lang::error::ErrorCode;

pub enum InstructionDecodeType {
    BaseHex,
    Base64,
    Base58,
}

#[derive(Debug)]
pub enum ChainInstructions {
    CreateAmmConfig {
        index: u16,
        trade_fee_rate: u64,
        protocol_fee_rate: u64,
        fund_fee_rate: u64,
        create_pool_fee: u64,
    },
    UpdateAmmConfig {
        param: u8,
        value: u64,
    },
    Initialize {
        token_0_mint: String,
        token_1_mint: String,
        init_amount_0: u64,
        init_amount_1: u64,
        open_time: u64,
    },
    UpdatePoolStatus {
        status: u8,
    },
    CollectProtocolFee {
        amount_0_requested: u64,
        amount_1_requested: u64,
    },
    CollectFundFee {
        amount_0_requested: u64,
        amount_1_requested: u64,
    },
    Deposit {
        lp_token_amount: u64,
        maximum_token_0_amount: u64,
        maximum_token_1_amount: u64,
    },
    Withdraw {
        lp_token_amount: u64,
        minimum_token_0_amount: u64,
        minimum_token_1_amount: u64,
    },
    SwapBaseInput {
        amount_in: u64,
        minimum_amount_out: u64,
    },
    SwapBaseOutput {
        max_amount_in: u64,
        amount_out: u64,
    },
}

// pub fn parse_program_event(
//     self_program_str: &str,
//     meta: Option<UiTransactionStatusMeta>,
// ) -> Result<(), ClientError> {
//     let logs: Vec<String> = if let Some(meta_data) = meta {
//         let log_messages = if let OptionSerializer::Some(log_messages) = meta_data.log_messages {
//             log_messages
//         } else {
//             Vec::new()
//         };
//         log_messages
//     } else {
//         Vec::new()
//     };
//     let mut logs = &logs[..];
//     if !logs.is_empty() {
//         if let Ok(mut execution) = Execution::new(&mut logs) {
//             for l in logs {
//                 let (new_program, did_pop) =
//                     if !execution.is_empty() && self_program_str == execution.program() {
//                         handle_program_log(self_program_str, &l, true).unwrap_or_else(|e| {
//                             println!("Unable to parse log: {e}");
//                             std::process::exit(1);
//                         })
//                     } else {
//                         let (program, did_pop) = handle_system_log(self_program_str, l);
//                         (program, did_pop)
//                     };
//                 // Switch program context on CPI.
//                 if let Some(new_program) = new_program {
//                     execution.push(new_program);
//                 }
//                 // Program returned.
//                 if did_pop {
//                     execution.pop();
//                 }
//             }
//         }
//     } else {
//         println!("log is empty");
//     }
//     Ok(())
// }

struct Execution {
    stack: Vec<String>,
}

impl Execution {
    pub fn new(logs: &mut &[String]) -> Result<Self, ClientError> {
        let l = &logs[0];
        *logs = &logs[1..];

        let re = Regex::new(r"^Program (.*) invoke.*$").unwrap();
        let c = re
            .captures(l)
            .ok_or_else(|| ClientError::LogParseError(l.to_string()))?;
        let program = c
            .get(1)
            .ok_or_else(|| ClientError::LogParseError(l.to_string()))?
            .as_str()
            .to_string();
        Ok(Self {
            stack: vec![program],
        })
    }

    pub fn program(&self) -> String {
        assert!(!self.stack.is_empty());
        self.stack[self.stack.len() - 1].clone()
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    pub fn push(&mut self, new_program: String) {
        self.stack.push(new_program);
    }

    pub fn pop(&mut self) {
        assert!(!self.stack.is_empty());
        self.stack.pop().unwrap();
    }
}

// pub fn handle_program_log(
//     self_program_str: &str,
//     l: &str,
//     with_prefix: bool,
// ) -> Result<(Option<String>, bool), ClientError> {
//     // Log emitted from the current program.
//     if let Some(log) = if with_prefix {
//         l.strip_prefix(PROGRAM_LOG)
//             .or_else(|| l.strip_prefix(PROGRAM_DATA))
//     } else {
//         Some(l)
//     } {
//         if l.starts_with(&format!("Program log:")) {
//             // not log event
//             return Ok((None, false));
//         }
//         let borsh_bytes = match anchor_lang::__private::base64::decode(log) {
//             Ok(borsh_bytes) => borsh_bytes,
//             _ => {
//                 println!("Could not base64 decode log: {}", log);
//                 return Ok((None, false));
//             }
//         };

//         let mut slice: &[u8] = &borsh_bytes[..];
//         let disc: [u8; 8] = {
//             let mut disc = [0; 8];
//             disc.copy_from_slice(&borsh_bytes[..8]);
//             slice = &slice[8..];
//             disc
//         };
//         match disc {
//             SwapEvent::DISCRIMINATOR => {
//                 println!("{:#?}", decode_event::<SwapEvent>(&mut slice)?);
//             }
//             LpChangeEvent::DISCRIMINATOR => {
//                 println!("{:#?}", decode_event::<LpChangeEvent>(&mut slice)?);
//             }
//             _ => {
//                 println!("unknow event: {}", l);
//             }
//         }
//         return Ok((None, false));
//     } else {
//         let (program, did_pop) = handle_system_log(self_program_str, l);
//         return Ok((program, did_pop));
//     }
// }

fn handle_system_log(this_program_str: &str, log: &str) -> (Option<String>, bool) {
    if log.starts_with(&format!("Program {this_program_str} invoke")) {
        (Some(this_program_str.to_string()), false)
    } else if log.contains("invoke") {
        (Some("cpi".to_string()), false) // Any string will do.
    } else {
        let re = Regex::new(r"^Program (.*) success*$").unwrap();
        if re.is_match(log) {
            (None, true)
        } else {
            (None, false)
        }
    }
}

fn decode_event<T: anchor_lang::Event + anchor_lang::AnchorDeserialize>(
    slice: &mut &[u8],
) -> Result<T, ClientError> {
    let event: T = anchor_lang::AnchorDeserialize::deserialize(slice)
        .map_err(|e| ClientError::LogParseError(e.to_string()))?;
    Ok(event)
}

pub fn parse_program_instruction(
    self_program_str: &str,
    encoded_transaction: EncodedTransaction,
    meta: Option<UiTransactionStatusMeta>,
) -> Result<Vec<ChainInstructions>, ClientError> {
    let mut chain_instructions = Vec::new();

    let ui_raw_msg = match encoded_transaction {
        solana_transaction_status::EncodedTransaction::Json(ui_tx) => match ui_tx.message {
            solana_transaction_status::UiMessage::Raw(ui_raw_msg) => ui_raw_msg,
            _ => solana_transaction_status::UiRawMessage {
                header: solana_sdk::message::MessageHeader::default(),
                account_keys: Vec::new(),
                recent_blockhash: "".to_string(),
                instructions: Vec::new(),
                address_table_lookups: None,
            },
        },
        _ => solana_transaction_status::UiRawMessage {
            header: solana_sdk::message::MessageHeader::default(),
            account_keys: Vec::new(),
            recent_blockhash: "".to_string(),
            instructions: Vec::new(),
            address_table_lookups: None,
        },
    };

    if let Some(meta) = meta {
        let mut account_keys = ui_raw_msg.account_keys;
        let meta = meta.clone();
        match meta.loaded_addresses {
            OptionSerializer::Some(addresses) => {
                let mut writeable_address = addresses.writable;
                let mut readonly_address = addresses.readonly;
                account_keys.append(&mut writeable_address);
                account_keys.append(&mut readonly_address);
            }
            _ => {}
        }
        let program_index = account_keys
            .iter()
            .position(|r| r == self_program_str)
            .unwrap();

        for (i, ui_compiled_instruction) in ui_raw_msg.instructions.iter().enumerate() {
            if (ui_compiled_instruction.program_id_index as usize) == program_index {
                let out_put = format!("instruction #{}", i + 1);
                println!("{}", out_put.gradient(Color::Green));
                let accounts: Vec<String> = ui_compiled_instruction
                    .accounts
                    .iter()
                    .map(|&index| account_keys[index as usize].clone())
                    .collect();

                match handle_program_instruction(
                    &ui_compiled_instruction.data,
                    InstructionDecodeType::Base58,
                    accounts,
                ) {
                    Ok(chain_instruction) => {
                        chain_instructions.push(chain_instruction);
                    }
                    Err(e) => {
                        eprintln!("Error decoding instruction: {}", e);
                        continue;
                    }
                }
            }
        }

        if let OptionSerializer::Some(inner_instructions) = meta.inner_instructions {
            for inner in inner_instructions {
                for (i, instruction) in inner.instructions.iter().enumerate() {
                    if let solana_transaction_status::UiInstruction::Compiled(
                        ui_compiled_instruction,
                    ) = instruction
                    {
                        if (ui_compiled_instruction.program_id_index as usize) == program_index {
                            let out_put =
                                format!("inner_instruction #{}.{}", inner.index + 1, i + 1);
                            println!("{}", out_put.gradient(Color::Green));
                            let accounts: Vec<String> = ui_compiled_instruction
                                .accounts
                                .iter()
                                .map(|&index| account_keys[index as usize].clone())
                                .collect();

                            match handle_program_instruction(
                                &ui_compiled_instruction.data,
                                InstructionDecodeType::Base58,
                                accounts,
                            ) {
                                Ok(chain_instruction) => {
                                    chain_instructions.push(chain_instruction);
                                }
                                Err(e) => {
                                    eprintln!("Error decoding inner instruction: {}", e);
                                    continue;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(chain_instructions)
}

pub fn handle_program_instruction(
    instr_data: &str,
    decode_type: InstructionDecodeType,
    accounts: Vec<String>,
) -> Result<ChainInstructions> {
    let data = match decode_type {
        InstructionDecodeType::BaseHex => hex::decode(instr_data).unwrap(),
        InstructionDecodeType::Base64 => match anchor_lang::__private::base64::decode(instr_data) {
            Ok(decoded) => decoded,
            Err(_) => {
                return Err(anyhow::anyhow!(ClientError::LogParseError(
                    "Could not base64 decode instruction".to_string()
                )))
            }
        },
        InstructionDecodeType::Base58 => match bs58::decode(instr_data).into_vec() {
            Ok(decoded) => decoded,
            Err(_) => {
                return Err(anyhow::anyhow!(ClientError::LogParseError(
                    "Could not base58 decode instruction".to_string()
                )))
            }
        },
    };

    let mut ix_data: &[u8] = &data[..];
    let disc: [u8; 8] = {
        let mut disc = [0; 8];
        disc.copy_from_slice(&data[..8]);
        ix_data = &ix_data[8..];
        disc
    };

    match disc {
        instruction::CreateAmmConfig::DISCRIMINATOR => {
            match decode_instruction::<instruction::CreateAmmConfig>(&mut ix_data) {
                Ok(ix) => Ok(ChainInstructions::CreateAmmConfig {
                    index: ix.index as u16,
                    trade_fee_rate: ix.token_0_creator_rate,
                    protocol_fee_rate: ix.token_1_lp_rate,
                    fund_fee_rate: ix.token_0_lp_rate,
                    create_pool_fee: ix.token_1_creator_rate,
                }),
                Err(e) => Err(anyhow::anyhow!(
                    "Failed to decode CreateAmmConfig instruction: {}",
                    e
                )),
            }
        }
        instruction::Initialize::DISCRIMINATOR => {
            match decode_instruction::<instruction::Initialize>(&mut ix_data) {
                Ok(ix) => Ok(ChainInstructions::Initialize {
                    token_0_mint: accounts[4].clone(),
                    token_1_mint: accounts[5].clone(),
                    init_amount_0: ix.init_amount_0,
                    init_amount_1: ix.init_amount_1,
                    open_time: ix.open_time,
                }),
                Err(e) => Err(anyhow::anyhow!(
                    "Failed to decode Initialize instruction: {}",
                    e
                )),
            }
        }
        instruction::Deposit::DISCRIMINATOR => {
            match decode_instruction::<instruction::Deposit>(&mut ix_data) {
                Ok(ix) => Ok(ChainInstructions::Deposit {
                    lp_token_amount: ix.lp_token_amount,
                    maximum_token_0_amount: ix.maximum_token_0_amount,
                    maximum_token_1_amount: ix.maximum_token_1_amount,
                }),
                Err(e) => Err(anyhow::anyhow!(
                    "Failed to decode Deposit instruction: {}",
                    e
                )),
            }
        }
        instruction::Withdraw::DISCRIMINATOR => {
            match decode_instruction::<instruction::Withdraw>(&mut ix_data) {
                Ok(ix) => Ok(ChainInstructions::Withdraw {
                    lp_token_amount: ix.lp_token_amount,
                    minimum_token_0_amount: ix.minimum_token_0_amount,
                    minimum_token_1_amount: ix.minimum_token_1_amount,
                }),
                Err(e) => Err(anyhow::anyhow!(
                    "Failed to decode Withdraw instruction: {}",
                    e
                )),
            }
        }
        instruction::SwapBaseInput::DISCRIMINATOR => {
            match decode_instruction::<instruction::SwapBaseInput>(&mut ix_data) {
                Ok(ix) => Ok(ChainInstructions::SwapBaseInput {
                    amount_in: ix.amount_in,
                    minimum_amount_out: ix.minimum_amount_out,
                }),
                Err(e) => Err(anyhow::anyhow!(
                    "Failed to decode SwapBaseInput instruction: {}",
                    e
                )),
            }
        }
        instruction::SwapBaseOutput::DISCRIMINATOR => {
            match decode_instruction::<instruction::SwapBaseOutput>(&mut ix_data) {
                Ok(ix) => Ok(ChainInstructions::SwapBaseOutput {
                    max_amount_in: ix.max_amount_in,
                    amount_out: ix.amount_out,
                }),
                Err(e) => Err(anyhow::anyhow!(
                    "Failed to decode SwapBaseOutput instruction: {}",
                    e
                )),
            }
        }
        _ => Err(anyhow::anyhow!("Unknown instruction")),
    }
}

fn decode_instruction<T: anchor_lang::AnchorDeserialize>(
    slice: &mut &[u8],
) -> Result<T, anchor_lang::error::ErrorCode> {
    let instruction: T = anchor_lang::AnchorDeserialize::deserialize(slice)
        .map_err(|_| anchor_lang::error::ErrorCode::InstructionDidNotDeserialize)?;
    Ok(instruction)
}

// pub fn handle_program_instruction(
//     instr_data: &str,
//     decode_type: InstructionDecodeType,
//     accounts: Vec<String>,
// ) -> Result<(), ClientError> {
//     let data;
//     match decode_type {
//         InstructionDecodeType::BaseHex => {
//             data = hex::decode(instr_data).unwrap();
//         }
//         InstructionDecodeType::Base64 => {
//             let borsh_bytes = match anchor_lang::__private::base64::decode(instr_data) {
//                 Ok(borsh_bytes) => borsh_bytes,
//                 _ => {
//                     println!("Could not base64 decode instruction: {}", instr_data);
//                     return Ok(());
//                 }
//             };
//             data = borsh_bytes;
//         }
//         InstructionDecodeType::Base58 => {
//             let borsh_bytes = match bs58::decode(instr_data).into_vec() {
//                 Ok(borsh_bytes) => borsh_bytes,
//                 _ => {
//                     println!("Could not base58 decode instruction: {}", instr_data);
//                     return Ok(());
//                 }
//             };
//             data = borsh_bytes;
//         }
//     }

//     let mut ix_data: &[u8] = &data[..];
//     let disc: [u8; 8] = {
//         let mut disc = [0; 8];
//         disc.copy_from_slice(&data[..8]);
//         ix_data = &ix_data[8..];
//         disc
//     };
//     // println!("{:?}", disc);

//     match disc {
//         instruction::CreateAmmConfig::DISCRIMINATOR => {
//             let ix = decode_instruction::<instruction::CreateAmmConfig>(&mut ix_data).unwrap();
//             #[derive(Debug)]
//             pub struct CreateAmmConfig {
//                 pub index: u64,
//                 token_1_lp_rate: u64,
//                 token_0_lp_rate: u64,
//                 token_0_creator_rate: u64,
//                 token_1_creator_rate: u64,
//             }
//             impl From<instruction::CreateAmmConfig> for CreateAmmConfig {
//                 fn from(instr: instruction::CreateAmmConfig) -> CreateAmmConfig {
//                     CreateAmmConfig {
//                         index: instr.index,
//                         token_1_lp_rate: ,
//                         token_0_lp_rate: ,
//                         token_0_creator_rate: ,
//                         token_1_creator_rate: ,
//                     }
//                 }
//             }
//             println!("{:#?}", CreateAmmConfig::from(ix));
//         }
//         instruction::Initialize::DISCRIMINATOR => {
//             let ix = decode_instruction::<instruction::Initialize>(&mut ix_data).unwrap();
//             #[derive(Debug)]
//             pub struct Initialize {
//                 pub token_0_mint: String,
//                 pub token_1_mint: String,
//                 pub init_amount_0: u64,
//                 pub init_amount_1: u64,
//                 pub open_time: u64,
//             }

//             let initialize = Initialize {
//                 token_0_mint: accounts[4].clone(),
//                 token_1_mint: accounts[5].clone(),
//                 init_amount_0: ix.init_amount_0,
//                 init_amount_1: ix.init_amount_1,
//                 open_time: ix.open_time,
//             };

//             println!("{:#?}", initialize);
//         }
//         instruction::Deposit::DISCRIMINATOR => {
//             let ix = decode_instruction::<instruction::Deposit>(&mut ix_data).unwrap();
//             #[derive(Debug)]
//             pub struct Deposit {
//                 pub lp_token_amount: u64,
//                 pub maximum_token_0_amount: u64,
//                 pub maximum_token_1_amount: u64,
//             }
//             impl From<instruction::Deposit> for Deposit {
//                 fn from(instr: instruction::Deposit) -> Deposit {
//                     Deposit {
//                         lp_token_amount: instr.lp_token_amount,
//                         maximum_token_0_amount: instr.maximum_token_0_amount,
//                         maximum_token_1_amount: instr.maximum_token_1_amount,
//                     }
//                 }
//             }
//             println!("{:#?}", Deposit::from(ix));
//         }
//         instruction::Withdraw::DISCRIMINATOR => {
//             let ix = decode_instruction::<instruction::Withdraw>(&mut ix_data).unwrap();
//             #[derive(Debug)]
//             pub struct Withdraw {
//                 pub lp_token_amount: u64,
//                 pub minimum_token_0_amount: u64,
//                 pub minimum_token_1_amount: u64,
//             }
//             impl From<instruction::Withdraw> for Withdraw {
//                 fn from(instr: instruction::Withdraw) -> Withdraw {
//                     Withdraw {
//                         lp_token_amount: instr.lp_token_amount,
//                         minimum_token_0_amount: instr.minimum_token_0_amount,
//                         minimum_token_1_amount: instr.minimum_token_1_amount,
//                     }
//                 }
//             }
//             println!("{:#?}", Withdraw::from(ix));
//         }
//         instruction::SwapBaseInput::DISCRIMINATOR => {
//             let ix = decode_instruction::<instruction::SwapBaseInput>(&mut ix_data).unwrap();
//             #[derive(Debug)]
//             pub struct SwapBaseInput {
//                 pub amount_in: u64,
//                 pub minimum_amount_out: u64,
//             }
//             impl From<instruction::SwapBaseInput> for SwapBaseInput {
//                 fn from(instr: instruction::SwapBaseInput) -> SwapBaseInput {
//                     SwapBaseInput {
//                         amount_in: instr.amount_in,
//                         minimum_amount_out: instr.minimum_amount_out,
//                     }
//                 }
//             }
//             println!("{:#?}", SwapBaseInput::from(ix));
//         }
//         instruction::SwapBaseOutput::DISCRIMINATOR => {
//             let ix = decode_instruction::<instruction::SwapBaseOutput>(&mut ix_data).unwrap();
//             #[derive(Debug)]
//             pub struct SwapBaseOutput {
//                 pub max_amount_in: u64,
//                 pub amount_out: u64,
//             }
//             impl From<instruction::SwapBaseOutput> for SwapBaseOutput {
//                 fn from(instr: instruction::SwapBaseOutput) -> SwapBaseOutput {
//                     SwapBaseOutput {
//                         max_amount_in: instr.max_amount_in,
//                         amount_out: instr.amount_out,
//                     }
//                 }
//             }
//             println!("{:#?}", SwapBaseOutput::from(ix));
//         }
//         _ => {
//             println!("unknow instruction: {}", instr_data);
//         } // instruction::UpdateAmmConfig::DISCRIMINATOR => {
//           //     let ix = decode_instruction::<instruction::UpdateAmmConfig>(&mut ix_data).unwrap();
//           //     #[derive(Debug)]
//           //     pub struct UpdateAmmConfig {
//           //         pub param: u8,
//           //         pub value: u64,
//           //     }
//           //     impl From<instruction::UpdateAmmConfig> for UpdateAmmConfig {
//           //         fn from(instr: instruction::UpdateAmmConfig) -> UpdateAmmConfig {
//           //             UpdateAmmConfig {
//           //                 param: instr.param,
//           //                 value: instr.value,
//           //             }
//           //         }
//           //     }
//           //     println!("{:#?}", UpdateAmmConfig::from(ix));
//           // }

//           // instruction::UpdatePoolStatus::DISCRIMINATOR => {
//           //     let ix = decode_instruction::<instruction::UpdatePoolStatus>(&mut ix_data).unwrap();
//           //     #[derive(Debug)]
//           //     pub struct UpdatePoolStatus {
//           //         pub status: u8,
//           //     }
//           //     impl From<instruction::UpdatePoolStatus> for UpdatePoolStatus {
//           //         fn from(instr: instruction::UpdatePoolStatus) -> UpdatePoolStatus {
//           //             UpdatePoolStatus {
//           //                 status: instr.status,
//           //             }
//           //         }
//           //     }
//           //     println!("{:#?}", UpdatePoolStatus::from(ix));
//           // }
//           // instruction::CollectProtocolFee::DISCRIMINATOR => {
//           //     let ix = decode_instruction::<instruction::CollectProtocolFee>(&mut ix_data).unwrap();
//           //     #[derive(Debug)]
//           //     pub struct CollectProtocolFee {
//           //         pub amount_0_requested: u64,
//           //         pub amount_1_requested: u64,
//           //     }
//           //     impl From<instruction::CollectProtocolFee> for CollectProtocolFee {
//           //         fn from(instr: instruction::CollectProtocolFee) -> CollectProtocolFee {
//           //             CollectProtocolFee {
//           //                 amount_0_requested: instr.amount_0_requested,
//           //                 amount_1_requested: instr.amount_1_requested,
//           //             }
//           //         }
//           //     }
//           //     println!("{:#?}", CollectProtocolFee::from(ix));
//           // }
//           // instruction::CollectFundFee::DISCRIMINATOR => {
//           //     let ix = decode_instruction::<instruction::CollectFundFee>(&mut ix_data).unwrap();
//           //     #[derive(Debug)]
//           //     pub struct CollectFundFee {
//           //         pub amount_0_requested: u64,
//           //         pub amount_1_requested: u64,
//           //     }
//           //     impl From<instruction::CollectFundFee> for CollectFundFee {
//           //         fn from(instr: instruction::CollectFundFee) -> CollectFundFee {
//           //             CollectFundFee {
//           //                 amount_0_requested: instr.amount_0_requested,
//           //                 amount_1_requested: instr.amount_1_requested,
//           //             }
//           //         }
//           //     }
//           //     println!("{:#?}", CollectFundFee::from(ix));
//           // }
//     }
//     Ok(())
// }
