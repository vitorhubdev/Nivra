//! Decoder-driven attachment ranges. No file cache or eager whole-video download.
use std::{
	collections::VecDeque,
	io::{self, Read, Seek, SeekFrom},
	sync::{
		Arc,
		atomic::{AtomicBool, AtomicUsize, Ordering},
	},
	time::Duration,
};
use tokio::runtime::Handle;

const MAX_BYTES: usize = 100 * 1024 * 1024;
const CHUNK: usize = 256 * 1024;
const CACHE_CHUNKS: usize = 8;
const INVALID: &str = "Video download failed or changed; reload the conversation";
const UNSUPPORTED: &str = "Video server does not support buffering; download to play externally";
const EXPIRED: &str = "Video link expired; reload the conversation";
const STALLED: &str = "Video buffering stalled; retry or download to play externally";
#[cfg(test)]
pub(super) static OFFLINE_PROBE: AtomicBool = AtomicBool::new(false);

pub(super) fn resolve_media_url(
	primary: url::Url,
	fallback: Option<url::Url>,
	cancelled: Arc<AtomicBool>,
	runtime: Handle,
) -> Result<url::Url, &'static str> {
	if cancelled.load(Ordering::Acquire) {
		return Err("Cancelled");
	}
	if !primary.username().is_empty() || primary.password().is_some() {
		return Err(INVALID);
	}
	let client = media_client()?;
	match probe(&client, &primary, &cancelled, &runtime) {
		Ok(_) => Ok(primary),
		Err(EXPIRED) => {
			let fallback = fallback.ok_or(EXPIRED)?;
			if !fallback.username().is_empty() || fallback.password().is_some() {
				return Err(INVALID);
			}
			probe(&client, &fallback, &cancelled, &runtime)?;
			Ok(fallback)
		}
		Err(error) => Err(error),
	}
}
fn media_client() -> Result<reqwest::Client, &'static str> {
	reqwest::Client::builder()
		.no_proxy()
		.no_gzip()
		.no_brotli()
		.no_deflate()
		.no_zstd()
		.redirect(reqwest::redirect::Policy::none())
		.http1_only()
		.timeout(Duration::from_secs(15))
		.build()
		.map_err(|_| INVALID)
}
fn probe(
	client: &reqwest::Client,
	url: &url::Url,
	cancelled: &AtomicBool,
	runtime: &Handle,
) -> Result<Probe, &'static str> {
	runtime.block_on(async {
		let cancelled_wait = async {
			while !cancelled.load(Ordering::Acquire) {
				tokio::time::sleep(Duration::from_millis(20)).await;
			}
		};
		let transfer = async {
			let response = tokio::time::timeout(
				Duration::from_secs(15),
				client
					.get(url.clone())
					.header(reqwest::header::ACCEPT_ENCODING, "identity")
					.header(reqwest::header::RANGE, "bytes=0-0")
					.send(),
			)
			.await
			.map_err(|_| STALLED)?
			.map_err(|_| INVALID)?;
			let status = response.status();
			if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::GONE {
				return Err(EXPIRED);
			}
			if status == reqwest::StatusCode::PARTIAL_CONTENT {
				// `Content-Range: bytes 0-0/12345` is the only length the server states
				// while also proving it serves byte ranges.
				let total = response
					.headers()
					.get(reqwest::header::CONTENT_RANGE)
					.and_then(|value| value.to_str().ok())
					.and_then(|value| value.rsplit_once('/'))
					.and_then(|(_, total)| total.trim().parse::<u64>().ok());
				return Ok(Probe {
					len: total.and_then(|total| usize::try_from(total).ok()),
					ranges: true,
				});
			}
			if status == reqwest::StatusCode::OK {
				// The server ignored the range, so it cannot stream: play from memory.
				return Ok(Probe {
					len: response
						.content_length()
						.and_then(|len| usize::try_from(len).ok()),
					ranges: false,
				});
			}
			Err(INVALID)
		};
		tokio::select! { biased;
			_ = cancelled_wait => Err("Cancelled"),
			result = transfer => result,
		}
	})
}

/// What one `Range: bytes=0-0` request told us about a media URL.
struct Probe {
	/// `None` when the server stated no length at all.
	len: Option<usize>,
	/// Whether the server honoured the range request.
	ranges: bool,
}

/// Whole-body download for servers without byte ranges, bounded by the same 100 MiB cap.
fn download_whole(
	client: &reqwest::Client,
	url: &url::Url,
	expected: usize,
	cancelled: &AtomicBool,
	runtime: &Handle,
) -> Result<Vec<u8>, &'static str> {
	runtime.block_on(async {
		let cancelled_wait = async {
			while !cancelled.load(Ordering::Acquire) {
				tokio::time::sleep(Duration::from_millis(20)).await;
			}
		};
		let transfer = async {
			let mut response = tokio::time::timeout(
				Duration::from_secs(15),
				client
					.get(url.clone())
					.header(reqwest::header::ACCEPT_ENCODING, "identity")
					.send(),
			)
			.await
			.map_err(|_| STALLED)?
			.map_err(|_| INVALID)?;
			if !response.status().is_success() {
				return Err(INVALID);
			}
			if response
				.headers()
				.get(reqwest::header::CONTENT_ENCODING)
				.is_some_and(|value| value != "identity")
			{
				return Err(INVALID);
			}
			let mut bytes = Vec::new();
			while let Some(chunk) = tokio::time::timeout(Duration::from_secs(15), response.chunk())
				.await
				.map_err(|_| STALLED)?
				.map_err(|_| INVALID)?
			{
				// Never grow past the length the server stated, whatever it sends.
				if bytes.len() + chunk.len() > expected {
					return Err(INVALID);
				}
				bytes.extend_from_slice(&chunk);
			}
			if bytes.is_empty() {
				return Err(INVALID);
			}
			Ok(bytes)
		};
		tokio::select! { biased;
			_ = cancelled_wait => Err("Cancelled"),
			result = transfer => result,
		}
	})
}

pub(super) fn source(
	url: Option<url::Url>,
	expected: usize,
	cancelled: Arc<AtomicBool>,
	log_budget: Arc<AtomicUsize>,
	runtime: Handle,
) -> Result<Box<dyn platform::video::ReadSeek>, &'static str> {
	let (input, len) = if let Some(url) = url {
		// The caller has validated the service URL; never let URL userinfo add credentials.
		if !url.username().is_empty() || url.password().is_some() {
			return Err(INVALID);
		}
		let client = media_client()?;
		// Embed previews carry no attachment size: ask the server once, then either
		// stream ranges or play the bounded whole body from memory.
		let (len, ranges) = if expected == 0 {
			if cancelled.load(Ordering::Acquire) {
				return Err("Cancelled");
			}
			let probe = probe(&client, &url, &cancelled, &runtime)?;
			let len = probe.len.filter(|len| *len > 0).ok_or(INVALID)?;
			if len > MAX_BYTES {
				return Err("Video preview limit: 100 MiB");
			}
			(len, probe.ranges)
		} else {
			if expected > MAX_BYTES {
				return Err("Video preview limit: 100 MiB");
			}
			(expected, true)
		};
		if ranges {
			(
				Input::Http {
					client,
					url,
					runtime,
				},
				len,
			)
		} else {
			let bytes = download_whole(&client, &url, len, &cancelled, &runtime)?;
			let len = bytes.len();
			(Input::Memory(bytes), len)
		}
	} else {
		if expected > MAX_BYTES {
			return Err("Video preview limit: 100 MiB");
		}
		#[cfg(not(feature = "demo"))]
		return Err(INVALID);
		#[cfg(feature = "demo")]
		{
			let bytes = include_bytes!("../../tests/fixtures/video.mov");
			if bytes.is_empty() || bytes.len() > MAX_BYTES {
				return Err(INVALID);
			}
			(Input::Demo(bytes), bytes.len())
		}
	};
	Ok(Box::new(Source {
		input,
		cancelled,
		log_budget,
		len,
		position: 0,
		cache: VecDeque::new(),
		start_time: std::time::Instant::now(),
		first_byte_logged: false,
	}))
}

enum Input {
	Http {
		client: reqwest::Client,
		url: url::Url,
		runtime: Handle,
	},
	/// Servers without byte ranges are buffered whole, still under the 100 MiB cap.
	Memory(Vec<u8>),
	#[cfg(feature = "demo")]
	Demo(&'static [u8]),
}
struct Source {
	input: Input,
	cancelled: Arc<AtomicBool>,
	log_budget: Arc<AtomicUsize>,
	len: usize,
	position: usize,
	cache: VecDeque<(usize, Vec<u8>)>,
	start_time: std::time::Instant,
	first_byte_logged: bool,
}
fn invalid() -> io::Error {
	io::Error::new(io::ErrorKind::InvalidData, INVALID)
}
fn stalled() -> io::Error {
	io::Error::new(io::ErrorKind::TimedOut, STALLED)
}
impl Source {
	fn check_cancelled(&self) -> io::Result<()> {
		if self.cancelled.load(Ordering::Acquire) {
			// Read::read_exact retries Interrupted indefinitely.
			Err(io::Error::new(
				io::ErrorKind::ConnectionAborted,
				"Cancelled",
			))
		} else {
			Ok(())
		}
	}
	fn fill(&mut self) -> io::Result<()> {
		// Retain both tracks across demuxer seeks. Evict before transfer so encoded
		// payloads, including the new range, stay within eight chunks / 2 MiB.
		if self.cache.len() == CACHE_CHUNKS {
			self.cache.pop_front();
		}
		let (client, url, runtime) = match &self.input {
			Input::Http {
				client,
				url,
				runtime,
			} => (client, url, runtime),
			Input::Memory(_) => return Err(invalid()),
			#[cfg(feature = "demo")]
			Input::Demo(_) => return Err(invalid()),
		};
		let start = self.position / CHUNK * CHUNK;
		let count = CHUNK.min(self.len - start);
		let end = start + count - 1;
		let bytes = runtime.block_on(async {
			let cancelled = async {
				while !self.cancelled.load(Ordering::Acquire) {
					tokio::time::sleep(Duration::from_millis(20)).await;
				}
			};
			let transfer = async {
				let mut response = tokio::time::timeout(
					Duration::from_secs(15),
					client
						.get(url.clone())
						.header(reqwest::header::ACCEPT_ENCODING, "identity")
						.header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
						.send(),
				)
				.await
				.map_err(|_| stalled())?
				.map_err(|_| invalid())?;
				if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
					let range = format!("bytes {start}-{end}/{}", self.len);
					if response
						.headers()
						.get(reqwest::header::CONTENT_RANGE)
						.and_then(|value| value.to_str().ok())
						!= Some(range.as_str())
					{
						return Err(invalid());
					}
				} else if !(response.status() == reqwest::StatusCode::OK
					&& start == 0 && count == self.len)
				{
					return Err(io::Error::new(io::ErrorKind::Unsupported, UNSUPPORTED));
				}
				if response
					.headers()
					.get(reqwest::header::CONTENT_ENCODING)
					.is_some_and(|value| value != "identity")
					|| response
						.content_length()
						.is_some_and(|len| len != count as u64)
				{
					return Err(invalid());
				}
				let mut bytes = Vec::with_capacity(count);
				while let Some(chunk) =
					tokio::time::timeout(Duration::from_secs(15), response.chunk())
						.await
						.map_err(|_| stalled())?
						.map_err(|_| invalid())?
				{
					if chunk.len() > count - bytes.len() {
						return Err(invalid());
					}
					bytes.extend_from_slice(&chunk);
				}
				if bytes.len() != count {
					return Err(invalid());
				}
				Ok(bytes)
			};
			tokio::select! { biased;
				_ = cancelled => Err(io::Error::new(io::ErrorKind::ConnectionAborted, "Cancelled")),
				result = transfer => result,
			}
		})?;
		self.check_cancelled()?;
		self.cache.push_back((start, bytes));
		Ok(())
	}
}
impl Read for Source {
	fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
		self.check_cancelled()?;
		let mut count = output.len().min(CHUNK).min(self.len - self.position);
		if count == 0 {
			return Ok(0);
		}
		#[cfg(feature = "demo")]
		if let Input::Demo(bytes) = self.input {
			output[..count].copy_from_slice(&bytes[self.position..self.position + count]);
			self.position += count;
			self.log_first_byte(count);
			return Ok(count);
		}
		if let Input::Memory(bytes) = &self.input {
			output[..count].copy_from_slice(&bytes[self.position..self.position + count]);
			self.position += count;
			self.log_first_byte(count);
			return Ok(count);
		}
		if let Some(index) = self.cache.iter().position(|(start, bytes)| {
			self.position >= *start && self.position < start + bytes.len()
		}) {
			let range = self.cache.remove(index).ok_or_else(invalid)?;
			self.cache.push_back(range);
		} else {
			self.fill()?;
		}
		let (start, bytes) = self.cache.back().ok_or_else(invalid)?;
		let offset = self.position - start;
		count = count.min(bytes.len() - offset);
		output[..count].copy_from_slice(&bytes[offset..offset + count]);
		self.position += count;
		self.log_first_byte(count);
		Ok(count)
	}
}
impl Source {
	/// First-byte timing measures data actually handed out (after any range
	/// request), not the moment the read was attempted (Codex PR #34 P2).
	fn log_first_byte(&mut self, count: usize) {
		if !self.first_byte_logged && count > 0 {
			self.first_byte_logged = true;
			vlog!(
				&self.log_budget,
				"first_byte: read in {:.3} ms",
				self.start_time.elapsed().as_secs_f64() * 1000.0
			);
		}
	}
}
impl Seek for Source {
	fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
		self.check_cancelled()?;
		let position = match position {
			SeekFrom::Start(position) => Some(position),
			SeekFrom::Current(offset) => (self.position as u64).checked_add_signed(offset),
			SeekFrom::End(offset) => (self.len as u64).checked_add_signed(offset),
		}
		.filter(|position| *position <= self.len as u64)
		.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid video seek"))?;
		self.position = position as usize;
		Ok(position)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::{io::Write, net::TcpListener, sync::atomic::AtomicUsize};
	#[test]
	fn first_byte_timing_waits_for_handed_out_data() {
		discord_api::ensure_tls_provider();
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let url = url::Url::parse(&format!(
			"http://{}/synthetic.mov",
			listener.local_addr().unwrap()
		))
		.unwrap();
		let server = std::thread::spawn(move || {
			let (mut socket, _) = listener.accept().unwrap();
			let mut header = Vec::new();
			while !header.ends_with(b"\r\n\r\n") {
				let mut byte = [0];
				if socket.read_exact(&mut byte).is_err() {
					return;
				}
				header.push(byte[0]);
			}
			// The range request fails: no data is ever handed out.
			write!(socket, "HTTP/1.1 500 Broken\r\nContent-Length: 0\r\n\r\n").unwrap();
		});
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(1)
			.enable_all()
			.build()
			.unwrap();
		let mut source = Source {
			input: Input::Http {
				client: media_client().unwrap(),
				url,
				runtime: runtime.handle().clone(),
			},
			cancelled: Arc::new(AtomicBool::new(false)),
			log_budget: Arc::new(AtomicUsize::new(usize::MAX)),
			len: CHUNK,
			position: 0,
			cache: VecDeque::new(),
			start_time: std::time::Instant::now(),
			first_byte_logged: false,
		};
		let mut chunk = vec![0; CHUNK];
		assert!(source.read(&mut chunk).is_err());
		assert!(
			!source.first_byte_logged,
			"failed reads hand out no data, so no first-byte timing"
		);
		server.join().unwrap();
	}

	#[test]
	fn alternating_tracks_reuse_buffered_ranges() {
		discord_api::ensure_tls_provider();
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let address = listener.local_addr().unwrap();
		let url = url::Url::parse(&format!("http://{address}/tracks.mov")).unwrap();
		let expected = 8 * 1024 * 1024;
		let requests = Arc::new(AtomicUsize::new(0));
		let observed = requests.clone();
		let stopped = Arc::new(AtomicBool::new(false));
		let stop = stopped.clone();
		let server = std::thread::spawn(move || {
			loop {
				let (mut socket, _) = listener.accept().unwrap();
				if stop.load(Ordering::Acquire) {
					break;
				}
				socket
					.set_read_timeout(Some(Duration::from_secs(2)))
					.unwrap();
				let mut header = Vec::new();
				while !header.ends_with(b"\r\n\r\n") {
					assert!(header.len() < 8192);
					let mut byte = [0];
					socket.read_exact(&mut byte).unwrap();
					header.push(byte[0]);
				}
				let header = String::from_utf8(header).unwrap().to_ascii_lowercase();
				let range = header
					.lines()
					.find_map(|line| line.strip_prefix("range: bytes="))
					.unwrap();
				let (start, end) = range.split_once('-').unwrap();
				let start = start.parse::<usize>().unwrap();
				let end = end.parse::<usize>().unwrap();
				let count = end - start + 1;
				assert!(count <= 256 * 1024 && end < expected);
				observed.fetch_add(1, Ordering::Release);
				std::thread::sleep(Duration::from_millis(10));
				write!(socket, "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{expected}\r\nContent-Length: {count}\r\nConnection: close\r\n\r\n").unwrap();
				socket
					.write_all(&vec![(start / (4 * 1024 * 1024)) as u8; count])
					.unwrap();
			}
		});
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(1)
			.enable_all()
			.build()
			.unwrap();
		let mut input = source(
			Some(url),
			expected,
			Arc::new(AtomicBool::new(false)),
			Arc::new(AtomicUsize::new(usize::MAX)),
			runtime.handle().clone(),
		)
		.unwrap();
		assert_eq!(requests.load(Ordering::Acquire), 0);
		let started = std::time::Instant::now();
		for sample in 0..32 {
			for track in 0..2 {
				input
					.seek(SeekFrom::Start(
						(track * 4 * 1024 * 1024 + sample * 4096) as u64,
					))
					.unwrap();
				let mut bytes = [0; 4096];
				input.read_exact(&mut bytes).unwrap();
				assert!(bytes.iter().all(|byte| *byte == track as u8));
			}
		}
		let elapsed = started.elapsed();
		let count = requests.load(Ordering::Acquire);
		stopped.store(true, Ordering::Release);
		std::net::TcpStream::connect(address).unwrap();
		server.join().unwrap();
		eprintln!(
			"alternating tracks: {count} requests, {elapsed:?}, 64 reads, 10 ms response delay"
		);
		assert_eq!(count, 2);
	}

	#[test]
	fn cancellation_interrupts_a_stalled_range_body() {
		discord_api::ensure_tls_provider();
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let url = url::Url::parse(&format!(
			"http://{}/stalled.mov",
			listener.local_addr().unwrap()
		))
		.unwrap();
		let cancelled = Arc::new(AtomicBool::new(false));
		let stop = cancelled.clone();
		let server = std::thread::spawn(move || {
			let (mut socket, _) = listener.accept().unwrap();
			socket
				.set_read_timeout(Some(Duration::from_secs(2)))
				.unwrap();
			let mut header = Vec::new();
			while !header.ends_with(b"\r\n\r\n") {
				assert!(header.len() < 8192);
				let mut byte = [0];
				socket.read_exact(&mut byte).unwrap();
				header.push(byte[0]);
			}
			write!(socket, "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-{}/{CHUNK}\r\nContent-Length: {CHUNK}\r\n\r\n", CHUNK - 1).unwrap();
			std::thread::sleep(Duration::from_millis(40));
			stop.store(true, Ordering::Release);
			// Keep the response open until cancellation drops the connection.
			match socket.read(&mut [0]) {
				Ok(0) => {}
				Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
				other => panic!("cancelled range connection stayed open: {other:?}"),
			}
		});
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(1)
			.enable_all()
			.build()
			.unwrap();
		let mut input = source(
			Some(url),
			CHUNK,
			cancelled,
			Arc::new(AtomicUsize::new(usize::MAX)),
			runtime.handle().clone(),
		)
		.unwrap();
		assert_eq!(
			input.read(&mut [0]).unwrap_err().kind(),
			io::ErrorKind::ConnectionAborted
		);
		server.join().unwrap();
	}

	#[test]
	fn ranges_follow_reads_and_seeks_and_reject_changed_responses() {
		discord_api::ensure_tls_provider();
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let url = url::Url::parse(&format!(
			"http://{}/synthetic.mov",
			listener.local_addr().unwrap()
		))
		.unwrap();
		let expected = CHUNK * (CACHE_CHUNKS + 1);
		let requests = Arc::new(AtomicUsize::new(0));
		let observed = requests.clone();
		let server = std::thread::spawn(move || {
			let starts = [0, CHUNK * CACHE_CHUNKS]
				.into_iter()
				.chain((1..CACHE_CHUNKS).map(|chunk| chunk * CHUNK))
				.chain([CHUNK * CACHE_CHUNKS]);
			for (index, start) in starts.enumerate() {
				let (mut socket, _) = listener.accept().unwrap();
				socket
					.set_read_timeout(Some(Duration::from_secs(2)))
					.unwrap();
				let mut header = Vec::new();
				while !header.ends_with(b"\r\n\r\n") {
					assert!(header.len() < 8192);
					let mut byte = [0];
					socket.read_exact(&mut byte).unwrap();
					header.push(byte[0]);
				}
				let header = String::from_utf8(header).unwrap().to_ascii_lowercase();
				assert!(
					header.contains(&format!("range: bytes={start}-{}\r\n", start + CHUNK - 1))
				);
				assert!(!header.contains("authorization:"));
				assert!(!header.contains("cookie:"));
				observed.fetch_add(1, Ordering::Release);
				let changed = index == CACHE_CHUNKS + 1;
				let total = expected + usize::from(changed);
				write!(socket, "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{}/{total}\r\nContent-Length: {CHUNK}\r\nConnection: close\r\n\r\n", start + CHUNK - 1).unwrap();
				let result = socket.write_all(&vec![(start / CHUNK) as u8; CHUNK]);
				if !changed {
					result.unwrap();
				}
			}
		});
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(1)
			.enable_all()
			.build()
			.unwrap();
		let cancelled = Arc::new(AtomicBool::new(false));
		let mut input = source(
			Some(url.clone()),
			expected,
			cancelled.clone(),
			Arc::new(AtomicUsize::new(usize::MAX)),
			runtime.handle().clone(),
		)
		.unwrap();
		assert_eq!(requests.load(Ordering::Acquire), 0);
		let mut bytes = [1; 32];
		input.read_exact(&mut bytes).unwrap();
		assert_eq!(bytes, [0; 32]);
		assert_eq!(requests.load(Ordering::Acquire), 1);
		input.seek(SeekFrom::Start(8)).unwrap();
		input.read_exact(&mut bytes).unwrap();
		assert_eq!(requests.load(Ordering::Acquire), 1);
		input.seek(SeekFrom::End(-32)).unwrap();
		input.read_exact(&mut bytes).unwrap();
		assert_eq!(bytes, [CACHE_CHUNKS as u8; 32]);
		assert_eq!(requests.load(Ordering::Acquire), 2);
		assert_eq!(input.read(&mut bytes).unwrap(), 0);
		assert!(input.seek(SeekFrom::End(1)).is_err());
		assert!(input.seek(SeekFrom::Start(u64::MAX)).is_err());
		assert!(input.seek(SeekFrom::Current(i64::MIN)).is_err());
		for chunk in 1..CACHE_CHUNKS {
			// Keep the first range hot while loading enough others to evict the tail.
			input.seek(SeekFrom::Start(0)).unwrap();
			input.read_exact(&mut bytes).unwrap();
			assert_eq!(bytes, [0; 32]);
			input.seek(SeekFrom::Start((chunk * CHUNK) as u64)).unwrap();
			input.read_exact(&mut bytes).unwrap();
			assert_eq!(bytes, [chunk as u8; 32]);
		}
		assert_eq!(requests.load(Ordering::Acquire), CACHE_CHUNKS + 1);
		input.seek(SeekFrom::Start(0)).unwrap();
		input.read_exact(&mut bytes).unwrap();
		assert_eq!(bytes, [0; 32]);
		input.seek(SeekFrom::End(-32)).unwrap();
		assert_eq!(
			input.read(&mut bytes).unwrap_err().kind(),
			io::ErrorKind::InvalidData
		);
		assert_eq!(requests.load(Ordering::Acquire), CACHE_CHUNKS + 2);
		server.join().unwrap();
		cancelled.store(true, Ordering::Release);
		assert_eq!(
			input.read(&mut bytes).unwrap_err().kind(),
			io::ErrorKind::ConnectionAborted
		);
		assert!(
			source(
				Some(url),
				MAX_BYTES + 1,
				cancelled,
				Arc::new(AtomicUsize::new(usize::MAX)),
				runtime.handle().clone()
			)
			.is_err()
		);
	}

	#[test]
	fn embed_video_without_a_size_streams_or_buffers_the_whole_file() {
		discord_api::ensure_tls_provider();
		for ranged in [true, false] {
			let listener = TcpListener::bind("127.0.0.1:0").unwrap();
			let address = listener.local_addr().unwrap();
			let url = url::Url::parse(&format!("http://{address}/external/clip.mp4")).unwrap();
			let clip = vec![7u8; 4096];
			let expected = clip.len();
			let body = clip.clone();
			let stop = Arc::new(AtomicBool::new(false));
			let stopped = stop.clone();
			let server = std::thread::spawn(move || {
				loop {
					let (mut socket, _) = match listener.accept() {
						Ok(pair) => pair,
						Err(_) => break,
					};
					if stopped.load(Ordering::Acquire) {
						break;
					}
					socket
						.set_read_timeout(Some(Duration::from_secs(2)))
						.unwrap();
					let mut header = Vec::new();
					while !header.ends_with(b"\r\n\r\n") && header.len() < 8192 {
						let mut byte = [0];
						if socket.read_exact(&mut byte).is_err() {
							break;
						}
						header.push(byte[0]);
					}
					let header = String::from_utf8_lossy(&header).to_ascii_lowercase();
					if ranged {
						let range = header
							.lines()
							.find_map(|line| line.strip_prefix("range: bytes="))
							.expect("ranged server answers ranges");
						let (start, end) = range.split_once('-').unwrap();
						let start: usize = start.parse().unwrap();
						let end: usize = end.parse().unwrap();
						assert!(end < expected, "{start}-{end} of {expected}");
						write!(socket, "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{expected}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n", end - start + 1).unwrap();
						socket.write_all(&body[start..=end]).unwrap();
					} else {
						// A server without range support ignores the probe's Range header
						// and always answers 200 with the whole file.
						write!(
							socket,
							"HTTP/1.1 200 OK\r\nContent-Length: {expected}\r\nConnection: close\r\n\r\n"
						)
						.unwrap();
						socket.write_all(&body).unwrap();
					}
				}
			});
			let runtime = tokio::runtime::Builder::new_multi_thread()
				.worker_threads(1)
				.enable_all()
				.build()
				.unwrap();
			let mut input = source(
				Some(url),
				0,
				Arc::new(AtomicBool::new(false)),
				Arc::new(AtomicUsize::new(usize::MAX)),
				runtime.handle().clone(),
			)
			.expect("embed preview without a size");
			assert_eq!(input.seek(SeekFrom::End(0)).unwrap(), expected as u64);
			input.seek(SeekFrom::Start(0)).unwrap();
			let mut head = [0; 4];
			input.read_exact(&mut head).unwrap();
			assert_eq!(head, [7; 4]);
			input.seek(SeekFrom::Start(2048)).unwrap();
			let mut middle = [0; 8];
			input.read_exact(&mut middle).unwrap();
			assert_eq!(middle, [7; 8]);
			drop(input);
			stop.store(true, Ordering::Release);
			let _ = std::net::TcpStream::connect(address);
			server.join().unwrap();
		}
	}

	#[test]
	fn embed_preview_without_a_length_is_refused() {
		discord_api::ensure_tls_provider();
		let listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let url = url::Url::parse(&format!(
			"http://{}/external/clip.mp4",
			listener.local_addr().unwrap()
		))
		.unwrap();
		let server = std::thread::spawn(move || {
			let (mut socket, _) = listener.accept().unwrap();
			socket
				.set_read_timeout(Some(Duration::from_secs(2)))
				.unwrap();
			let mut header = Vec::new();
			while !header.ends_with(b"\r\n\r\n") && header.len() < 8192 {
				let mut byte = [0];
				if socket.read_exact(&mut byte).is_err() {
					return;
				}
				header.push(byte[0]);
			}
			// 206 without a parsable total length: nothing bounded to stream.
			write!(socket, "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-0/*\r\nContent-Length: 1\r\nConnection: close\r\n\r\n").unwrap();
		});
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(1)
			.enable_all()
			.build()
			.unwrap();
		assert!(
			source(
				Some(url),
				0,
				Arc::new(AtomicBool::new(false)),
				Arc::new(AtomicUsize::new(usize::MAX)),
				runtime.handle().clone()
			)
			.is_err()
		);
		server.join().unwrap();
	}

	#[test]
	fn expired_link_uses_the_proxy_url() {
		discord_api::ensure_tls_provider();
		let primary = TcpListener::bind("127.0.0.1:0").unwrap();
		let fallback_listener = TcpListener::bind("127.0.0.1:0").unwrap();
		let primary_url = url::Url::parse(&format!(
			"http://{}/attachments/1/2/clip.mp4?ex=1&is=1&hm=abc&backend=b2",
			primary.local_addr().unwrap()
		))
		.unwrap();
		let fallback_url = url::Url::parse(&format!(
			"http://{}/attachments/1/2/clip.mp4?ex=1&is=1&hm=abc",
			fallback_listener.local_addr().unwrap()
		))
		.unwrap();
		let expected = fallback_url.clone();
		let server = std::thread::spawn(move || {
			fn answer(listener: TcpListener, body: &str) {
				let (mut socket, _) = listener.accept().unwrap();
				socket
					.set_read_timeout(Some(Duration::from_secs(2)))
					.unwrap();
				let mut header = Vec::new();
				while !header.ends_with(b"\r\n\r\n") && header.len() < 8192 {
					let mut byte = [0];
					if socket.read_exact(&mut byte).is_err() {
						return;
					}
					header.push(byte[0]);
				}
				let header = String::from_utf8_lossy(&header).to_ascii_lowercase();
				assert!(header.contains("range: bytes=0-0"), "{header}");
				write!(socket, "{body}").unwrap();
				let _ = socket.shutdown(std::net::Shutdown::Both);
			}
			answer(
				primary,
				"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
			);
			answer(
				fallback_listener,
				"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-0/1\r\nContent-Length: 1\r\nConnection: close\r\n\r\nX",
			);
		});
		let runtime = tokio::runtime::Builder::new_multi_thread()
			.worker_threads(1)
			.enable_all()
			.build()
			.unwrap();
		let chosen = resolve_media_url(
			primary_url,
			Some(fallback_url),
			Arc::new(AtomicBool::new(false)),
			runtime.handle().clone(),
		)
		.unwrap();
		assert_eq!(chosen.as_str(), expected.as_str());
		server.join().unwrap();
	}
}
