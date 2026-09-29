//! Effective per-attachment upload ceiling from account Nitro tier and guild boost tier.
//!
//! Account caps: <https://support.discord.com/hc/en-us/articles/33694251638295> (File Sharing Limit row).
//! Server caps: same article (Upload row under Servers).
//! Default per attachment when no boost/Nitro applies: 10 MiB
//! (<https://github.com/discord/discord-api-docs/blob/main/developers/reference.mdx> — Uploading Files).

pub const MIB: u64 = 1024 * 1024;

const ACCOUNT_DEFAULT: u64 = 10 * MIB;
const ACCOUNT_BASIC: u64 = 50 * MIB;
const ACCOUNT_NITRO: u64 = 500 * MIB;

const GUILD_DEFAULT: u64 = 10 * MIB;
const GUILD_TIER1: u64 = 10 * MIB;
const GUILD_TIER2: u64 = 50 * MIB;
const GUILD_TIER3: u64 = 100 * MIB;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UploadLimitSource {
	Account,
	ServerBoost,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct UploadLimit {
	pub bytes: u64,
	pub source: UploadLimitSource,
}

pub fn account_upload_bytes(premium_type: Option<u8>) -> u64 {
	match premium_type {
		Some(2) => ACCOUNT_NITRO,
		Some(1 | 3) => ACCOUNT_BASIC,
		_ => ACCOUNT_DEFAULT,
	}
}

pub fn guild_upload_bytes(premium_tier: u8) -> u64 {
	match premium_tier {
		3 => GUILD_TIER3,
		2 => GUILD_TIER2,
		1 => GUILD_TIER1,
		_ => GUILD_DEFAULT,
	}
}

/// Maximum bytes allowed for one attachment in this context (greater of account and server).
pub fn upload_limit(premium_type: Option<u8>, guild_premium_tier: u8) -> UploadLimit {
	let account = account_upload_bytes(premium_type);
	let guild = guild_upload_bytes(guild_premium_tier);
	if guild > account {
		UploadLimit {
			bytes: guild,
			source: UploadLimitSource::ServerBoost,
		}
	} else if account > guild {
		UploadLimit {
			bytes: account,
			source: UploadLimitSource::Account,
		}
	} else if guild_premium_tier > 0 {
		UploadLimit {
			bytes: guild,
			source: UploadLimitSource::ServerBoost,
		}
	} else {
		UploadLimit {
			bytes: account,
			source: UploadLimitSource::Account,
		}
	}
}

pub fn exceeds_limit(limit: UploadLimit, sizes: &[u64]) -> bool {
	sizes.iter().any(|&size| size > limit.bytes)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn account_limits_by_premium_type() {
		assert_eq!(account_upload_bytes(None), ACCOUNT_DEFAULT);
		assert_eq!(account_upload_bytes(Some(0)), ACCOUNT_DEFAULT);
		assert_eq!(account_upload_bytes(Some(3)), ACCOUNT_BASIC);
		assert_eq!(account_upload_bytes(Some(1)), ACCOUNT_BASIC);
		assert_eq!(account_upload_bytes(Some(2)), ACCOUNT_NITRO);
	}

	#[test]
	fn guild_limits_by_boost_tier() {
		assert_eq!(guild_upload_bytes(0), GUILD_DEFAULT);
		assert_eq!(guild_upload_bytes(1), GUILD_TIER1);
		assert_eq!(guild_upload_bytes(2), GUILD_TIER2);
		assert_eq!(guild_upload_bytes(3), GUILD_TIER3);
	}

	#[test]
	fn upload_limit_is_max_of_account_and_guild() {
		let none = upload_limit(None, 0);
		assert_eq!(none.bytes, ACCOUNT_DEFAULT);
		assert_eq!(none.source, UploadLimitSource::Account);

		let nitro = upload_limit(Some(2), 0);
		assert_eq!(nitro.bytes, ACCOUNT_NITRO);
		assert_eq!(nitro.source, UploadLimitSource::Account);

		let boost3 = upload_limit(None, 3);
		assert_eq!(boost3.bytes, GUILD_TIER3);
		assert_eq!(boost3.source, UploadLimitSource::ServerBoost);

		let nitro_on_boost3 = upload_limit(Some(2), 3);
		assert_eq!(nitro_on_boost3.bytes, ACCOUNT_NITRO);
		assert_eq!(nitro_on_boost3.source, UploadLimitSource::Account);

		let basic_on_boost2 = upload_limit(Some(3), 2);
		assert_eq!(basic_on_boost2.bytes, GUILD_TIER2);
		assert_eq!(basic_on_boost2.source, UploadLimitSource::ServerBoost);
	}

	#[test]
	fn exceeds_limit_flags_oversized_files() {
		let limit = upload_limit(None, 0);
		assert!(!exceeds_limit(limit, &[limit.bytes, 1]));
		assert!(exceeds_limit(limit, &[limit.bytes + 1]));
	}
}
