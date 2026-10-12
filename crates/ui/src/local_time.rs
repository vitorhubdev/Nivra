//! Converts UTC instants to the user's local zone for display.

/// Shifts `instant` to the local offset in effect at that moment; falls back to UTC.
pub fn local(instant: time::OffsetDateTime) -> time::OffsetDateTime {
	// Tests pin UTC so date-boundary assertions hold on every machine.
	let offset = if cfg!(test) {
		time::UtcOffset::UTC
	} else {
		time::UtcOffset::local_offset_at(instant).unwrap_or(time::UtcOffset::UTC)
	};
	instant.to_offset(offset)
}

/// Current wall-clock time in the local zone.
pub fn now() -> time::OffsetDateTime {
	local(time::OffsetDateTime::now_utc())
}

/// Message headers include the local calendar date once the message is 24 hours old.
pub(crate) fn message_timestamp(instant: time::OffsetDateTime) -> String {
	message_timestamp_at(local(instant), time::OffsetDateTime::now_utc())
}

fn message_timestamp_at(at: time::OffsetDateTime, now: time::OffsetDateTime) -> String {
	let clock = format!("{:02}:{:02}", at.hour(), at.minute());
	if now - at >= time::Duration::hours(24) {
		format!(
			"{:02}/{:02}/{:04} {clock}",
			at.day(),
			u8::from(at.month()),
			at.year()
		)
	} else {
		clock
	}
}

/// A full local timestamp with its actual offset, including for grouped hover labels.
pub(crate) fn message_timestamp_tooltip(at: time::OffsetDateTime) -> String {
	format!(
		"{:02}/{:02}/{:04} {:02}:{:02}:{:02} UTC{}",
		at.day(),
		u8::from(at.month()),
		at.year(),
		at.hour(),
		at.minute(),
		at.second(),
		at.offset()
	)
}

/// Renders a Discord `<t:seconds:style>` reference the way the official client does: absolute
/// styles in the viewer's zone, `R` as a coarse relative phrase that refreshes on every frame.
pub fn discord_timestamp(seconds: i64, style: u8) -> Option<String> {
	let instant = time::OffsetDateTime::from_unix_timestamp(seconds).ok()?;
	if style == b'R' {
		return Some(relative(instant - time::OffsetDateTime::now_utc()));
	}
	let at = local(instant);
	let date = at.date();
	let clock = format!("{:02}:{:02}", at.hour(), at.minute());
	Some(match style {
		b't' => clock,
		b'T' => format!("{clock}:{:02}", at.second()),
		b'd' => format!(
			"{:02}/{:02}/{}",
			u8::from(date.month()),
			date.day(),
			date.year()
		),
		b'D' => format!("{} {}, {}", date.month(), date.day(), date.year()),
		b'F' => format!(
			"{}, {} {}, {} {clock}",
			date.weekday(),
			date.month(),
			date.day(),
			date.year()
		),
		_ => format!("{} {}, {} {clock}", date.month(), date.day(), date.year()),
	})
}
/// "… ago" for a past instant, measured against the current clock.
pub(crate) fn ago(instant: time::OffsetDateTime) -> String {
	relative(instant - time::OffsetDateTime::now_utc())
}
/// Coarse "in …"/"… ago" phrasing with the same thresholds the web client's relative times use.
fn relative(delta: time::Duration) -> String {
	let seconds = delta.whole_seconds();
	let ahead = seconds > 0;
	let seconds = seconds.unsigned_abs();
	// Rounded like the web client: 47 hours reads "2 days", not "1 day".
	let minutes = (seconds + 30) / 60;
	let hours = (minutes + 30) / 60;
	let days = (hours + 12) / 24;
	let months = (days + 15) / 30;
	let years = (days + 182) / 365;
	let amount = if seconds < 45 {
		"a few seconds".to_owned()
	} else if seconds < 90 {
		"a minute".to_owned()
	} else if minutes < 45 {
		format!("{minutes} minutes")
	} else if minutes < 90 {
		"an hour".to_owned()
	} else if hours < 22 {
		format!("{hours} hours")
	} else if hours < 36 {
		"a day".to_owned()
	} else if days < 26 {
		format!("{days} days")
	} else if days < 46 {
		"a month".to_owned()
	} else if days < 320 {
		format!("{months} months")
	} else if days < 548 {
		"a year".to_owned()
	} else {
		format!("{years} years")
	};
	if ahead {
		format!("in {amount}")
	} else {
		format!("{amount} ago")
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn message_date_appears_at_exactly_twenty_four_hours() {
		let at = time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
		for (age, expected) in [
			(
				time::Duration::hours(24) - time::Duration::nanoseconds(1),
				"22:13",
			),
			(time::Duration::hours(24), "14/11/2023 22:13"),
			(time::Duration::hours(48), "14/11/2023 22:13"),
			(time::Duration::hours(-1), "22:13"),
		] {
			assert_eq!(super::message_timestamp_at(at, at + age), expected);
		}
	}

	#[test]
	fn message_date_and_clock_use_the_viewers_offset_but_age_uses_the_instant() {
		// 00:30 UTC on January 1 is still December 31 for a viewer in Brazil.
		let utc = time::OffsetDateTime::from_unix_timestamp(1_735_691_400).unwrap();
		let brazil = utc.to_offset(time::UtcOffset::from_hms(-3, 0, 0).unwrap());
		let ahead = utc.to_offset(time::UtcOffset::from_hms(14, 0, 0).unwrap());
		let now = utc + time::Duration::hours(24);
		assert_eq!(super::message_timestamp_at(brazil, now), "31/12/2024 21:30");
		assert_eq!(super::message_timestamp_at(ahead, now), "01/01/2025 14:30");
		assert_eq!(
			super::message_timestamp_at(brazil, now - time::Duration::seconds(1)),
			"21:30"
		);
		assert_eq!(
			super::message_timestamp_tooltip(brazil),
			"31/12/2024 21:30:00 UTC-03:00:00"
		);
	}

	#[test]
	fn formats_every_discord_timestamp_style() {
		let at = 1_700_000_000;
		for (style, expected) in [
			(b't', "22:13"),
			(b'T', "22:13:20"),
			(b'd', "11/14/2023"),
			(b'D', "November 14, 2023"),
			(b'f', "November 14, 2023 22:13"),
			(b'F', "Tuesday, November 14, 2023 22:13"),
		] {
			assert_eq!(
				super::discord_timestamp(at, style).as_deref(),
				Some(expected)
			);
		}
		assert_eq!(super::discord_timestamp(i64::MAX, b'f'), None);
	}
	#[test]
	fn phrases_relative_timestamps_in_both_directions() {
		let now = time::OffsetDateTime::now_utc().unix_timestamp();
		for (offset, expected) in [
			(-5, "a few seconds ago"),
			(-600, "10 minutes ago"),
			(-7_200, "2 hours ago"),
			(-864_000, "10 days ago"),
			(-15_552_000, "6 months ago"),
			(-157_680_000, "5 years ago"),
			(3_600, "in an hour"),
			(172_800, "in 2 days"),
		] {
			assert_eq!(
				super::discord_timestamp(now + offset, b'R').as_deref(),
				Some(expected),
				"offset {offset}"
			);
		}
	}
	#[test]
	fn keeps_the_instant() {
		let utc = time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
		assert_eq!(super::local(utc).unix_timestamp(), utc.unix_timestamp());
	}
}
