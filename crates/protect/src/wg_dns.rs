//! Minimal DNS A lookups for the in-process WireGuard engine. Names resolve only through the
//! tunnel, so a hostname never reaches the operating system's resolver on this path.
use std::net::Ipv4Addr;

/// Standard recursive A query. Labels are 1..=63 ASCII bytes; the whole name is at most 253.
pub(super) fn query(name: &str, id: u16) -> Option<Vec<u8>> {
	let name = name.strip_suffix('.').unwrap_or(name);
	if name.is_empty() || name.len() > 253 || !name.is_ascii() {
		return None;
	}
	let mut packet = Vec::with_capacity(name.len() + 18);
	packet.extend_from_slice(&id.to_be_bytes());
	// Flags: recursion desired. One question, no other records.
	packet.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
	for label in name.split('.') {
		if label.is_empty() || label.len() > 63 {
			return None;
		}
		packet.push(label.len() as u8);
		packet.extend_from_slice(label.as_bytes());
	}
	packet.push(0);
	// Type A, class IN.
	packet.extend_from_slice(&[0, 1, 0, 1]);
	Some(packet)
}

/// First IPv4 address in the answer section of a successful response to `id`.
pub(super) fn answer(reply: &[u8], id: u16) -> Option<Ipv4Addr> {
	let header = reply.get(..12)?;
	if u16::from_be_bytes([header[0], header[1]]) != id {
		return None;
	}
	let flags = u16::from_be_bytes([header[2], header[3]]);
	// Must be a response with rcode NOERROR.
	if flags & 0x8000 == 0 || flags & 0x000f != 0 {
		return None;
	}
	let questions = u16::from_be_bytes([header[4], header[5]]);
	let answers = u16::from_be_bytes([header[6], header[7]]);
	let mut at = 12;
	for _ in 0..questions {
		at = skip_name(reply, at)?.checked_add(4)?;
	}
	for _ in 0..answers {
		at = skip_name(reply, at)?;
		let kind = u16_at(reply, at)?;
		let length = u16_at(reply, at.checked_add(8)?)? as usize;
		let data = at.checked_add(10)?;
		if kind == 1 && length == 4 {
			let octets = reply.get(data..data + 4)?;
			return Some(Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]));
		}
		at = data.checked_add(length)?;
	}
	None
}

/// Offset just after a name, following either inline labels or one compression pointer.
fn skip_name(reply: &[u8], mut at: usize) -> Option<usize> {
	// Bounded: a valid name has at most 127 labels.
	for _ in 0..128 {
		let length = *reply.get(at)?;
		if length & 0xc0 == 0xc0 {
			reply.get(at + 1)?;
			return at.checked_add(2);
		}
		if length & 0xc0 != 0 {
			return None;
		}
		if length == 0 {
			return at.checked_add(1);
		}
		at = at.checked_add(1 + length as usize)?;
	}
	None
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
	let pair = bytes.get(at..at.checked_add(2)?)?;
	Some(u16::from_be_bytes([pair[0], pair[1]]))
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A response to `id` for `example.com` with one compressed A record, 93.184.216.34.
	fn reply(id: u16) -> Vec<u8> {
		let mut bytes = id.to_be_bytes().to_vec();
		bytes.extend_from_slice(&[0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0]);
		bytes.extend_from_slice(&[
			7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 3, b'c', b'o', b'm', 0,
		]);
		bytes.extend_from_slice(&[0, 1, 0, 1]);
		bytes.extend_from_slice(&[
			0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0x0e, 0x10, 0, 4, 93, 184, 216, 34,
		]);
		bytes
	}

	#[test]
	fn query_encodes_one_a_question() {
		let packet = query("example.com.", 0x1234).unwrap();
		assert_eq!(&packet[..2], &[0x12, 0x34]);
		assert_eq!(&packet[4..6], &[0, 1]);
		assert!(packet.ends_with(&[0, 0, 1, 0, 1]));
		assert!(query("", 1).is_none());
		assert!(query("a..b", 1).is_none());
		assert!(query(&"a".repeat(64), 1).is_none());
		assert!(query("bücher.de", 1).is_none());
	}

	#[test]
	fn answer_reads_the_first_a_record_for_the_matching_id_only() {
		assert_eq!(answer(&reply(7), 7), Some(Ipv4Addr::new(93, 184, 216, 34)));
		assert_eq!(answer(&reply(7), 8), None);
	}

	#[test]
	fn answer_rejects_errors_and_truncated_replies() {
		let mut failed = reply(7);
		failed[3] = 0x83; // NXDOMAIN
		assert_eq!(answer(&failed, 7), None);
		let truncated = &reply(7)[..20];
		assert_eq!(answer(truncated, 7), None);
		assert_eq!(answer(&[0; 5], 0), None);
	}
}
