//! Deterministic zip archives: entries sorted by path, every timestamp the
//! zip epoch, and fixed permissions, so the same files always make the same
//! bytes. Stores and reviewers can then check a package against its source.

use std::io::Write as _;
use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use flate2::Compression;
use flate2::write::DeflateEncoder;

/// 1980-01-01 00:00, the earliest time a zip entry can carry.
const DOS_DATE: u16 = (1 << 5) | 1;
const DOS_TIME: u16 = 0;

/// Every file under `dir`, as `(path with / separators, contents)`, sorted.
pub fn files_under(dir: &Path) -> Result<Vec<(String, Vec<u8>)>> {
	let mut files = Vec::new();
	for entry in walkdir::WalkDir::new(dir).sort_by_file_name() {
		let entry = entry.with_context(|| format!("could not read {}", dir.display()))?;
		if !entry.file_type().is_file() {
			continue;
		}
		let relative = entry
			.path()
			.strip_prefix(dir)
			.expect("walkdir yields paths under its root");
		let name = relative
			.components()
			.map(|part| part.as_os_str().to_string_lossy().into_owned())
			.collect::<Vec<_>>()
			.join("/");
		let contents = std::fs::read(entry.path())
			.with_context(|| format!("could not read {}", entry.path().display()))?;
		files.push((name, contents));
	}
	files.sort_by(|left, right| left.0.cmp(&right.0));
	Ok(files)
}

/// A zip of `files`, which must already be sorted and unique.
pub fn archive(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
	let mut out = Vec::new();
	let mut central = Vec::new();
	for (name, contents) in files {
		let crc = crc32fast::hash(contents);
		let mut encoder = DeflateEncoder::new(Vec::new(), Compression::best());
		encoder.write_all(contents)?;
		let deflated = encoder.finish()?;
		// Store a file whose compressed form is no smaller.
		let (method, data): (u16, &[u8]) = if deflated.len() < contents.len() {
			(8, &deflated)
		} else {
			(0, contents)
		};
		let offset = u32::try_from(out.len()).context("the archive is over 4 GiB")?;
		let name_len = u16::try_from(name.len()).context("a path is too long for zip")?;
		let compressed = u32::try_from(data.len()).context("a file is over 4 GiB")?;
		let size = u32::try_from(contents.len()).context("a file is over 4 GiB")?;

		// Local file header.
		out.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
		out.extend_from_slice(&20_u16.to_le_bytes()); // version needed
		out.extend_from_slice(&0x0800_u16.to_le_bytes()); // UTF-8 names
		out.extend_from_slice(&method.to_le_bytes());
		out.extend_from_slice(&DOS_TIME.to_le_bytes());
		out.extend_from_slice(&DOS_DATE.to_le_bytes());
		out.extend_from_slice(&crc.to_le_bytes());
		out.extend_from_slice(&compressed.to_le_bytes());
		out.extend_from_slice(&size.to_le_bytes());
		out.extend_from_slice(&name_len.to_le_bytes());
		out.extend_from_slice(&0_u16.to_le_bytes()); // extra field
		out.extend_from_slice(name.as_bytes());
		out.extend_from_slice(data);

		// Central directory entry.
		central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
		central.extend_from_slice(&((3_u16 << 8) | 0x14).to_le_bytes()); // made by Unix, 2.0
		central.extend_from_slice(&20_u16.to_le_bytes());
		central.extend_from_slice(&0x0800_u16.to_le_bytes());
		central.extend_from_slice(&method.to_le_bytes());
		central.extend_from_slice(&DOS_TIME.to_le_bytes());
		central.extend_from_slice(&DOS_DATE.to_le_bytes());
		central.extend_from_slice(&crc.to_le_bytes());
		central.extend_from_slice(&compressed.to_le_bytes());
		central.extend_from_slice(&size.to_le_bytes());
		central.extend_from_slice(&name_len.to_le_bytes());
		central.extend_from_slice(&0_u16.to_le_bytes()); // extra field
		central.extend_from_slice(&0_u16.to_le_bytes()); // comment
		central.extend_from_slice(&0_u16.to_le_bytes()); // disk
		central.extend_from_slice(&0_u16.to_le_bytes()); // internal attributes
		central.extend_from_slice(&(0o100_644_u32 << 16).to_le_bytes()); // rw-r--r--
		central.extend_from_slice(&offset.to_le_bytes());
		central.extend_from_slice(name.as_bytes());
	}
	let central_offset = u32::try_from(out.len()).context("the archive is over 4 GiB")?;
	let central_size = u32::try_from(central.len()).context("the archive is over 4 GiB")?;
	let count = u16::try_from(files.len()).context("too many files for zip")?;
	out.extend_from_slice(&central);
	out.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
	out.extend_from_slice(&0_u16.to_le_bytes());
	out.extend_from_slice(&0_u16.to_le_bytes());
	out.extend_from_slice(&count.to_le_bytes());
	out.extend_from_slice(&count.to_le_bytes());
	out.extend_from_slice(&central_size.to_le_bytes());
	out.extend_from_slice(&central_offset.to_le_bytes());
	out.extend_from_slice(&0_u16.to_le_bytes());
	Ok(out)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_same_files_make_the_same_bytes() {
		let files = vec![
			("a.txt".to_owned(), b"hello hello hello hello".to_vec()),
			("pkg/b.bin".to_owned(), vec![1, 2, 3]),
		];
		assert_eq!(archive(&files).unwrap(), archive(&files).unwrap());
	}

	#[test]
	fn archives_unpack_with_the_system_unzip() {
		let directory = tempfile::tempdir().unwrap();
		let files = vec![
			(
				"dir/text.txt".to_owned(),
				b"some text, some text, some text".to_vec(),
			),
			("raw".to_owned(), vec![0, 255, 7]),
		];
		let zip = directory.path().join("out.zip");
		std::fs::write(&zip, archive(&files).unwrap()).unwrap();
		let Ok(status) = std::process::Command::new("unzip")
			.args(["-q", "-o"])
			.arg(&zip)
			.arg("-d")
			.arg(directory.path().join("out"))
			.status()
		else {
			return; // no unzip on this machine
		};
		assert!(status.success());
		assert_eq!(
			std::fs::read(directory.path().join("out/dir/text.txt")).unwrap(),
			b"some text, some text, some text"
		);
		assert_eq!(
			std::fs::read(directory.path().join("out/raw")).unwrap(),
			[0, 255, 7]
		);
	}
}
