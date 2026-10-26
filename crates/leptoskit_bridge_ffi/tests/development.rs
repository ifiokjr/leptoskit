#![cfg(feature = "app")]
#![allow(unsafe_code)]

//! The development guest ABI against a canned HTTP server: a new guest, an
//! unchanged one, a server failure, and invalid input.

use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::net::TcpListener;
use std::ptr;
use std::slice;

use leptoskit_bridge_ffi::Buffer;
use leptoskit_bridge_ffi::development::*;
use leptoskit_bridge_ffi::lk_buffer_release;
use leptoskit_codepush::hash_bytes;

fn take(buffer: Buffer) -> Vec<u8> {
	// SAFETY: Each owned output buffer is copied then released exactly once.
	unsafe {
		let bytes = slice::from_raw_parts(buffer.data, buffer.len).to_vec();
		lk_buffer_release(buffer);
		bytes
	}
}

/// Answers one request per canned response, in order, recording each
/// request line.
fn serve(responses: Vec<Vec<u8>>) -> (String, std::thread::JoinHandle<Vec<String>>) {
	let listener = TcpListener::bind("127.0.0.1:0").unwrap();
	let url = format!(
		"http://{}/_leptoskit/dev/ios/guest",
		listener.local_addr().unwrap()
	);
	let server = std::thread::spawn(move || {
		let mut requests = Vec::new();

		for response in responses {
			let (mut stream, _) = listener.accept().unwrap();
			let mut reader = BufReader::new(stream.try_clone().unwrap());
			let mut line = String::new();
			reader.read_line(&mut line).unwrap();
			requests.push(line.trim_end().to_owned());

			loop {
				let mut header = String::new();
				reader.read_line(&mut header).unwrap();

				if header == "\r\n" {
					break;
				}
			}

			stream.write_all(&response).unwrap();
		}

		requests
	});
	(url, server)
}

fn guest(wasm: &[u8]) -> Vec<u8> {
	let mut response = format!(
		"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: \"{}\"\r\nConnection: close\r\n\r\n",
		wasm.len(),
		hash_bytes(wasm)
	)
	.into_bytes();
	response.extend_from_slice(wasm);
	response
}

fn open(url: &str, running: &[u8]) -> Result<*mut Development, String> {
	let mut error = std::mem::MaybeUninit::<Buffer>::uninit();
	// SAFETY: Both inputs are borrowed for the call; `error` is writable.
	let development = unsafe {
		lk_dev_open(
			url.as_ptr(),
			url.len(),
			running.as_ptr(),
			running.len(),
			error.as_mut_ptr(),
		)
	};

	if development.is_null() {
		// SAFETY: A null result wrote the diagnostic buffer.
		return Err(String::from_utf8(take(unsafe { error.assume_init() })).unwrap());
	}

	Ok(development)
}

/// One `lk_dev_next` call: its status, and the guest when it returned one.
fn next(development: *mut Development) -> (i32, Option<Vec<u8>>) {
	let mut wasm = std::mem::MaybeUninit::<Buffer>::uninit();
	// SAFETY: The owner is null or live on this thread; the output is writable.
	let status = unsafe { lk_dev_next(development, wasm.as_mut_ptr()) };
	// SAFETY: Only a status of 1 writes the output buffer.
	let guest = (status == 1).then(|| take(unsafe { wasm.assume_init() }));
	(status, guest)
}

#[test]
fn an_app_receives_a_new_guest_waits_on_an_unchanged_one_and_reports_failures() {
	let (url, server) = serve(vec![
		guest(b"\0asm\x01\0\0\0new"),
		b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n".to_vec(),
		b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
			.to_vec(),
	]);
	let development = open(&url, b"embedded").unwrap();
	assert_eq!(next(development), (1, Some(b"\0asm\x01\0\0\0new".to_vec())));
	assert_eq!(next(development), (0, None));
	assert_eq!(next(development), (-1, None));

	// SAFETY: The owner is live on this thread until it is destroyed here.
	unsafe {
		let error = String::from_utf8(take(lk_dev_error(development))).unwrap();
		assert!(error.contains("HTTP 500"), "{error}");
		lk_dev_destroy(development);
	}

	let requests = server.join().unwrap();
	assert_eq!(
		requests[0],
		format!(
			"GET /_leptoskit/dev/ios/guest?after={} HTTP/1.1",
			hash_bytes(b"embedded")
		),
		"the first request starts from the embedded guest"
	);
	assert!(
		requests[1].contains(&hash_bytes(b"\0asm\x01\0\0\0new")),
		"later requests start from the guest just received: {}",
		requests[1]
	);
}

#[test]
fn invalid_urls_and_owners_fail_with_a_diagnostic() {
	let error = open("ftp://127.0.0.1/guest", b"").unwrap_err();
	assert!(error.contains("invalid development guest URL"), "{error}");
	let error = open("http://127.0.0.1/guest?after=x", b"").unwrap_err();
	assert!(error.contains("invalid development guest URL"), "{error}");

	assert_eq!(next(ptr::null_mut()), (-1, None));
	// SAFETY: A null owner is rejected before any access.
	unsafe {
		let error = String::from_utf8(take(lk_dev_error(ptr::null_mut()))).unwrap();
		assert_eq!(error, "invalid development guest owner");
		lk_dev_destroy(ptr::null_mut());
	}
}
