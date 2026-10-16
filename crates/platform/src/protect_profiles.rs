//! Bounded opaque PROTECT secrets in the OS credential store; no file fallback.
use crate::CredentialError;
use zeroize::Zeroizing;

const CHUNK: usize = 1024;
const CHUNKS: usize = 8;

fn entry(id: &str, index: usize) -> Result<keyring::Entry, CredentialError> {
	if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) || index >= CHUNKS {
		return Err(CredentialError::Invalid);
	}
	crate::entry(&format!("protect.{id}.{index}"))
}

/// IDs must be fresh. Immutable generations keep interrupted writes from corrupting a profile.
pub fn save(id: &str, bytes: &[u8]) -> Result<(), CredentialError> {
	if bytes.is_empty() || bytes.len() > CHUNK * CHUNKS {
		return Err(CredentialError::Invalid);
	}
	for (index, chunk) in bytes.chunks(CHUNK).enumerate() {
		if entry(id, index)?.set_secret(chunk).is_err() {
			let _ = forget(id);
			return Err(CredentialError::Unavailable);
		}
	}
	Ok(())
}

pub fn load(id: &str) -> Result<Zeroizing<Vec<u8>>, CredentialError> {
	let mut bytes = Zeroizing::new(Vec::with_capacity(CHUNK * CHUNKS));
	for index in 0..CHUNKS {
		match entry(id, index)?.get_secret() {
			Ok(value) => {
				let chunk = Zeroizing::new(value);
				if chunk.is_empty() || chunk.len() > CHUNK {
					return Err(CredentialError::Invalid);
				}
				bytes.extend_from_slice(&chunk);
				if chunk.len() < CHUNK {
					break;
				}
			}
			Err(keyring::Error::NoEntry) if index > 0 => break,
			Err(_) => return Err(CredentialError::Unavailable),
		}
	}
	Ok(bytes)
}

pub fn forget(id: &str) -> Result<(), CredentialError> {
	let mut result = Ok(());
	for index in 0..CHUNKS {
		match entry(id, index)?.delete_credential() {
			Ok(()) | Err(keyring::Error::NoEntry) => {}
			Err(_) => result = Err(CredentialError::Unavailable),
		}
	}
	result
}
