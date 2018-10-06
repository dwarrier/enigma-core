use rlp::{RlpStream, UntrustedRlp};
use hexutil::read_hex;
use std::str::from_utf8;
use std::string::ToString;
use std::mem;
use rlp::DecoderError;
use std::vec::Vec;
use std::string::String;
use cryptography_t::symmetric::{decrypt, encrypt};
use common::utils_t::{ToHex, FromHex};
use evm_t::get_key;
use common::errors_t::EnclaveError;
use bigint::U256;

enum SolidityType {
    Uint,
    String,
    Address,
    Bool,
    Bytes,
}

fn get_type(type_str: &str) -> SolidityType {
    let t = match &type_str[0..4] {
        "uint" => SolidityType::Uint,
        "addr" => SolidityType::Address,
        "stri" => SolidityType::String,
        "bool" => SolidityType::Bool,
        _ => SolidityType::Bytes,
    };
    t
}

fn convert_undecrypted_value_to_string(rlp: &UntrustedRlp, arg_type: &SolidityType) -> Result<String, EnclaveError> {
    let rlp_error: String = "Bad RLP encoding".to_string();
    let mut result: String = "".to_string();
    match arg_type {
        &SolidityType::String => {
            let string_result: Result<String, DecoderError> = rlp.as_val();
            result = match string_result {
                Ok(v) => v,
                Err(_e) => return Err(EnclaveError::InputError { message: rlp_error }),
            }
        },
        &SolidityType::Uint => {
            let num_result: Result<Vec<u8>, DecoderError> = rlp.as_val();
            result = match num_result {
                Ok(v) => {
                    complete_to_u256(v.to_hex())
                },
                Err(_e) => return Err(EnclaveError::InputError { message: rlp_error }),
            }
        },
        &SolidityType::Bool => {
            let num_result: Result<bool, DecoderError> = rlp.as_val();
            result = match num_result {
                Ok(v) => v.to_string(),
                Err(_e) => return Err(EnclaveError::InputError { message: rlp_error }),
            }
        },
        _ => {
            let bytes_result: Result<Vec<u8>, DecoderError> = rlp.as_val();
            match bytes_result {
                Ok(v) => {
                    match arg_type {
                        &SolidityType::Address => {
                            let string_result: Result<String, DecoderError> = rlp.as_val();
                            result = match string_result {
                                Ok(v) => v,
                                Err(_e) => v[..].to_hex(),
                            };
                            if result.starts_with("0x") {
                                result.remove(0);
                                result.remove(0);
                            }
                        },
                        _ => {
                            let iter = v.into_iter();
                            for item in iter {
                                result.push(item as char);
                            }
                        },
                    }
                },
                Err(_e) => return Err(EnclaveError::InputError { message: rlp_error }),
            };
        }
    }
    Ok(result)
}

pub fn complete_to_u256(num: String) -> String {
    let mut result: String = "".to_string();
    for i in num.len()..64 {
        result.push('0');
    }
    result.push_str(&num);
    result
}


fn decrypt_rlp(v: &[u8], key: &[u8], arg_type: &SolidityType) -> Result<String, EnclaveError> {
    let encrypted_value = match from_utf8(&v){
        Ok(value) => value,
        Err(e) => return Err(EnclaveError::InputError { message: "".to_string() }),
    };
    match read_hex(encrypted_value) {
        Err(e) => Err(EnclaveError::InputError { message: "".to_string() }),
        Ok(v) => {
            let decrypted_value = decrypt(&v, key);
            match decrypted_value {
                Ok(v) => { //The value is decrypted
                    let iter = v.clone().into_iter();
                    let mut decrypted_str = "".to_string();
                    //Remove 0x from the beginning, if used in encryption
                    match arg_type {
                        &SolidityType::Address => {
                            for item in iter {
                                decrypted_str.push(item as char);
                            }
                            if decrypted_str.starts_with("0x") {
                                decrypted_str.remove(0);
                                decrypted_str.remove(0);
                            }
                        },
                        &SolidityType::Uint => decrypted_str = complete_to_u256(v.to_hex()),
                        &SolidityType::Bool => {
                            let mut static_type_num= [0u8; 1];
                            static_type_num[..v.len()].clone_from_slice(&v);
                            let bool_val = unsafe { mem::transmute::<[u8; 1], bool>(static_type_num) };
                            decrypted_str = bool_val.to_string();
                        },

                        _ => {
                            for item in iter {
                                decrypted_str.push(item as char);
                            }

                        }
                    };
                    Ok(decrypted_str)
                }
                Err(e) => Err(e),
            }
        }
    }
}

fn decode_rlp(rlp: &UntrustedRlp, result: &mut String, key: &[u8], arg_type: &SolidityType) -> Result<(), EnclaveError> {
    if rlp.is_list() {
        result.push_str("[");
        let iter = rlp.iter();
        for item in iter {
            decode_rlp(&item, result, key, arg_type);
            result.push_str(",");
        }
//Replace the last ',' with ']'
        result.pop();
        result.push_str("]");
        Ok(())
    } else {
//Maybe the value is encrypted
        let as_val: Result<Vec<u8>, DecoderError> = rlp.as_val();
        let value: String = match as_val {
            Ok(v) => {
                match decrypt_rlp(&v, key, arg_type) {
                    Ok(result_string) => result_string,
                    Err(_e) => {
                        match convert_undecrypted_value_to_string(rlp, arg_type) {
                            Ok(result_string) => result_string,
                            Err(e) => return Err(e),
                        }
                    }
                }
            }
            Err(_e) => {
                match convert_undecrypted_value_to_string(rlp, arg_type) {
                    Ok(result_string) => result_string,
                    Err(e) => return Err(e),
                }
            }
        };
        result.push_str(&value);
        Ok(())
    }
}

pub fn decode_args(encoded: &[u8], types: &Vec<String>) -> Result<Vec<String>, EnclaveError> {
    let key = get_key();
    let rlp = UntrustedRlp::new(encoded);
    let mut result: Vec<String> = vec![];
    let iter = rlp.iter();
    let mut types_iter = types.iter();
    for item in iter {
        let mut str: String = "".to_string();
        let next_type = match types_iter.next() {
            Some(v) => v,
            None => return Err(EnclaveError::InputError { message: "Arguments and callable signature do not match".to_string() }),
        };
        match decode_rlp(&item, &mut str, &key, &get_type(next_type)) {
            Ok(_v) => result.push(str),
            Err(e) => return Err(e),
        };
    }
    Ok(result)
}
