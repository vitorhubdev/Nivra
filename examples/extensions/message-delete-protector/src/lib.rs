use nivra_extension_sdk::{Invocation, Output};

fn activate(_input: Invocation) -> Output {
	Output::default()
}

nivra_extension_sdk::export!(activate);
