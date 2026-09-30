use crate::{Error, ExtensionKind, Invocation, MAX_IO_BYTES, MAX_MODULE_BYTES, Output, Package};
use wasmi::{
	Config, EnforcedLimits, Engine, ExternType, FuncType, Instance, Linker, Module, Store,
	StoreLimits, StoreLimitsBuilder, TrapCode, ValType,
};

const MEMORY_BYTES: usize = 16 * 1024 * 1024;
const FUEL: u64 = 10_000_000;

fn engine() -> Engine {
	let mut config = Config::default();
	config
		.consume_fuel(true)
		.allow_start_fn(false)
		.ignore_custom_sections(true)
		.set_max_recursion_depth(128)
		.set_min_stack_height(4096)
		.set_max_stack_height(256 * 1024)
		.set_max_cached_stacks(0)
		.enforced_limits(EnforcedLimits::strict());
	Engine::new(&config)
}

/// Preferred (Nivra) and legacy (Serein) Wasm export names. Same signatures.
const ALLOC_NAMES: [&str; 2] = ["nivra_alloc", "serein_alloc"];
const INVOKE_NAMES: [&str; 2] = ["nivra_invoke", "serein_invoke"];

fn has_export(module: &Module, names: &[&str], ty: &FuncType) -> bool {
	names.iter().any(
		|name| matches!(module.get_export(name), Some(ExternType::Func(actual)) if actual == *ty),
	)
}

fn module(engine: &Engine, bytes: &[u8]) -> Result<Module, Error> {
	if bytes.len() > MAX_MODULE_BYTES {
		return Err(Error::Limit);
	}
	let module = Module::new(engine, bytes).map_err(|_| Error::Module)?;
	if module.imports().next().is_some() {
		return Err(Error::Module);
	}
	if !matches!(module.get_export("memory"), Some(ExternType::Memory(_)))
		|| !has_export(
			&module,
			&ALLOC_NAMES,
			&FuncType::new([ValType::I32], [ValType::I32]),
		) || !has_export(
		&module,
		&INVOKE_NAMES,
		&FuncType::new([ValType::I32, ValType::I32], [ValType::I64]),
	) {
		return Err(Error::Module);
	}
	Ok(module)
}

pub(crate) fn validate_module(bytes: &[u8]) -> Result<(), Error> {
	let engine = engine();
	let module = module(&engine, bytes)?;
	instantiate(&engine, &module).map(|_| ())
}

// Classify only engine-owned codes: never surface Wasm-provided text or input/output bytes.
fn execution_error(error: wasmi::Error) -> Error {
	match error.as_trap_code() {
		Some(TrapCode::OutOfFuel) => Error::Fuel,
		Some(TrapCode::GrowthOperationLimited | TrapCode::OutOfSystemMemory) => Error::Memory,
		Some(TrapCode::StackOverflow) => Error::Stack,
		Some(_) => Error::Trap,
		None => match error.kind() {
			wasmi::errors::ErrorKind::Memory(_) | wasmi::errors::ErrorKind::Table(_) => {
				Error::Memory
			}
			_ => Error::Execution,
		},
	}
}

fn instantiate(engine: &Engine, module: &Module) -> Result<(Store<StoreLimits>, Instance), Error> {
	let limits = StoreLimitsBuilder::new()
		.memory_size(MEMORY_BYTES)
		.memories(1)
		.tables(1)
		.table_elements(4096)
		.instances(1)
		.trap_on_grow_failure(true)
		.build();
	let mut store = Store::new(engine, limits);
	store.limiter(|limits| limits);
	store.set_fuel(FUEL).map_err(|_| Error::Execution)?;
	let instance = Linker::new(engine)
		.instantiate_and_start(&mut store, module)
		.map_err(execution_error)?;
	Ok((store, instance))
}

/// Execute once, on the host worker. All Wasm state is dropped before returning.
/// ABI: `nivra_alloc(i32) -> i32`, `nivra_invoke(i32, i32) -> i64`, with a
/// fallback to the legacy `serein_alloc` / `serein_invoke` (same signatures).
/// The result packs the output pointer in its high 32 bits and byte length in its low 32 bits.
pub fn invoke(package: &Package, input: &Invocation) -> Result<Output, Error> {
	if package.manifest.kind != ExtensionKind::Plugin || package.theme.is_some() {
		return Err(Error::Invalid);
	}
	package.manifest.validate()?;
	input
		.validate(&package.manifest)
		.map_err(|error| match error {
			Error::Invalid => Error::Input,
			Error::Limit => Error::InputLimit,
			other => other,
		})?;
	#[derive(serde::Serialize)]
	struct HostInvocation<'a> {
		#[serde(flatten)]
		input: &'a Invocation,
		host: crate::HostInfo,
	}
	let bytes = serde_json::to_vec(&HostInvocation {
		input,
		host: crate::HostInfo::current(),
	})
	.map_err(|_| Error::Input)?;
	if bytes.len() > MAX_IO_BYTES {
		return Err(Error::InputLimit);
	}
	let engine = engine();
	let module = module(&engine, &package.wasm)?;
	let (mut store, instance) = instantiate(&engine, &module)?;
	let memory = instance.get_memory(&store, "memory").ok_or(Error::Module)?;
	let alloc = ALLOC_NAMES
		.iter()
		.find_map(|name| instance.get_typed_func::<i32, i32>(&store, name).ok())
		.ok_or(Error::Module)?;
	let run = INVOKE_NAMES
		.iter()
		.find_map(|name| {
			instance
				.get_typed_func::<(i32, i32), i64>(&store, name)
				.ok()
		})
		.ok_or(Error::Module)?;
	let pointer = alloc
		.call(&mut store, bytes.len() as i32)
		.map_err(execution_error)?;
	memory
		.write(&mut store, pointer as u32 as usize, &bytes)
		.map_err(|_| Error::Trap)?;
	let packed = run
		.call(&mut store, (pointer, bytes.len() as i32))
		.map_err(execution_error)? as u64;
	let length = packed as u32 as usize;
	if length == 0 {
		return Err(Error::Handler);
	}
	if length > MAX_IO_BYTES {
		return Err(Error::OutputLimit);
	}
	let mut bytes = vec![0; length];
	memory
		.read(&store, (packed >> 32) as usize, &mut bytes)
		.map_err(|_| Error::Output)?;
	let output: Output = serde_json::from_slice(&bytes).map_err(|_| Error::Output)?;
	output
		.validate(&package.manifest, input)
		.map_err(|error| match error {
			Error::Limit => Error::OutputLimit,
			Error::Invalid => Error::Output,
			other => other,
		})?;
	Ok(output)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn module_wat(alloc: &str, invoke: &str) -> Vec<u8> {
		wat::parse_str(format!(
			"(module (memory 1) (export \"memory\" (memory 0)) (func (export \"{alloc}\") (param i32) (result i32) (i32.const 0)) \
			 (func (export \"{invoke}\") (param i32 i32) (result i64) (i64.const 0)))"
		))
		.expect("test wat")
	}

	#[test]
	fn accepts_nivra_alloc_invoke_first_and_serein_as_legacy() {
		// New-style module (nivra_*) must validate; old code only knew serein_*.
		validate_module(&module_wat("nivra_alloc", "nivra_invoke")).expect("nivra exports");
		// Legacy module (serein_*) keeps loading via fallback.
		validate_module(&module_wat("serein_alloc", "serein_invoke")).expect("serein exports");
		// Both exported: still valid (prefers nivra at call time).
		let both = wat::parse_str(
			"(module (memory 1) (export \"memory\" (memory 0)) \
			 (func (export \"nivra_alloc\") (param i32) (result i32) (i32.const 0)) \
			 (func (export \"serein_alloc\") (param i32) (result i32) (i32.const 0)) \
			 (func (export \"nivra_invoke\") (param i32 i32) (result i64) (i64.const 0)) \
			 (func (export \"serein_invoke\") (param i32 i32) (result i64) (i64.const 0)))",
		)
		.expect("test wat");
		validate_module(&both).expect("both exports");
		// Neither / wrong signature: still rejected.
		assert!(validate_module(&module_wat("other_alloc", "nivra_invoke")).is_err());
		assert!(validate_module(&module_wat("nivra_alloc", "other_invoke")).is_err());
	}
}
