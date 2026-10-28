//! Bounded reproducer for RUSTSEC-2026-0414. No network or live account is used.
use pulldown_cmark::{Options, Parser};

#[test]
fn empty_wikilink_labels_do_not_multiply_the_remaining_events() {
	for repetitions in [1, 4, 16, 64] {
		let markdown = "[[a|]] ".repeat(repetitions);
		// The patched parser emits 4*n + 1 events. Stop early on an affected
		// parser so this regression itself can never exhaust the test runner.
		let limit = 4 * repetitions + 16;
		let count = Parser::new_ext(&markdown, Options::ENABLE_WIKILINKS)
			.take(limit + 1)
			.count();
		assert!(
			count <= limit,
			"wikilinks expanded beyond a linear bound: repetitions={repetitions}, events={count}"
		);
	}
}
