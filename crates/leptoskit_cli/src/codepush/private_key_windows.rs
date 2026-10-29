//! Atomic owner-only key creation. Updating an inherited ACL after opening the
//! file would let another user keep a readable handle before the secret is
//! written.

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::os::windows::io::FromRawHandle;
use std::os::windows::io::OwnedHandle;
use std::path::Path;
use std::ptr;

use anyhow::Context;
use anyhow::Result;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::Authorization::*;
use windows_sys::Win32::Security::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Threading::*;

/// Owns a security API allocation, including on error paths.
struct LocalMemory(*mut c_void);

impl Drop for LocalMemory {
	fn drop(&mut self) {
		// SAFETY: these pointers are returned by APIs requiring LocalFree.
		unsafe { LocalFree(self.0) };
	}
}

/// Creates a new file with a protected DACL granting only the current user
/// access.
pub(super) fn create(path: &Path) -> Result<File> {
	let path: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
	anyhow::ensure!(
		!path[..path.len() - 1].contains(&0),
		"key path contains a NUL"
	);

	// SAFETY: all outputs have aligned, live storage; API-owned buffers and
	// handles are kept alive until their final use and released by their owning
	// guards.
	unsafe {
		let mut token = ptr::null_mut();
		check(OpenProcessToken(
			GetCurrentProcess(),
			TOKEN_QUERY,
			&raw mut token,
		))?;
		let token = OwnedHandle::from_raw_handle(token);
		const USER_BYTES: usize = size_of::<TOKEN_USER>() + SECURITY_MAX_SID_SIZE as usize;
		let mut user = [0usize; USER_BYTES.div_ceil(size_of::<usize>())];
		let mut length = 0;
		check(GetTokenInformation(
			token.as_raw_handle(),
			TokenUser,
			user.as_mut_ptr().cast(),
			USER_BYTES as u32,
			&raw mut length,
		))?;
		let sid = (*user.as_ptr().cast::<TOKEN_USER>()).User.Sid;
		let mut sid_string = ptr::null_mut();
		check(ConvertSidToStringSidW(sid, &raw mut sid_string))?;
		let _sid_string = LocalMemory(sid_string.cast());
		let mut sid_length = 0;

		while *sid_string.add(sid_length) != 0 {
			sid_length += 1;
		}

		let sid_text = String::from_utf16(std::slice::from_raw_parts(sid_string, sid_length))?;
		// Set the owner explicitly: an elevated token's default owner can be a
		// group. P disables parent inheritance, including a shared directory's
		// allow rules.
		let sddl: Vec<u16> = format!("O:{sid_text}D:P(A;;FA;;;{sid_text})")
			.encode_utf16()
			.chain([0])
			.collect();
		let mut descriptor = ptr::null_mut();
		check(ConvertStringSecurityDescriptorToSecurityDescriptorW(
			sddl.as_ptr(),
			SDDL_REVISION_1,
			&raw mut descriptor,
			ptr::null_mut(),
		))?;
		let descriptor = LocalMemory(descriptor);
		let attributes = SECURITY_ATTRIBUTES {
			nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
			lpSecurityDescriptor: descriptor.0,
			bInheritHandle: 0,
		};
		let handle = CreateFileW(
			path.as_ptr(),
			GENERIC_WRITE | READ_CONTROL | DELETE,
			0,
			&raw const attributes,
			CREATE_NEW,
			FILE_ATTRIBUTE_NORMAL,
			ptr::null_mut(),
		);

		if handle == INVALID_HANDLE_VALUE {
			return Err(io::Error::last_os_error().into());
		}

		let file = File::from_raw_handle(handle);

		if let Err(error) = verify_owner_only(&file, sid) {
			// Delete by handle while it is still exclusively open, without a
			// path race.
			let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
			check(SetFileInformationByHandle(
				handle,
				FileDispositionInfo,
				(&raw const disposition).cast(),
				size_of::<FILE_DISPOSITION_INFO>() as u32,
			))
			.context("could not remove the empty key file after its ACL check failed")?;

			return Err(error.context("owner-only key permissions could not be verified"));
		}

		Ok(file)
	}
}

/// Rejects filesystems that ignore security descriptors or inherited
/// permissions.
unsafe fn verify_owner_only(file: &File, user: PSID) -> Result<()> {
	let mut descriptor = ptr::null_mut();
	let status = unsafe {
		GetSecurityInfo(
			file.as_raw_handle(),
			SE_FILE_OBJECT,
			OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
			ptr::null_mut(),
			ptr::null_mut(),
			ptr::null_mut(),
			ptr::null_mut(),
			&raw mut descriptor,
		)
	};
	let descriptor = LocalMemory(descriptor);

	if status != ERROR_SUCCESS {
		return Err(io::Error::from_raw_os_error(status as i32).into());
	}

	// SAFETY: the descriptor and caller's SID remain alive for this check.
	unsafe { verify_descriptor(descriptor.0, user) }
}

/// Requires an explicit owner, protected DACL and exactly one owner-only allow
/// ACE.
unsafe fn verify_descriptor(descriptor: PSECURITY_DESCRIPTOR, user: PSID) -> Result<()> {
	// SAFETY: both arguments are valid security API objects. All pointers below
	// borrow their storage from the descriptor for the duration of this check.
	unsafe {
		let mut owner = ptr::null_mut();
		let mut dacl = ptr::null_mut();
		let mut present = 0;
		let mut defaulted = 0;
		let mut control = 0;
		let mut revision = 0;
		check(GetSecurityDescriptorOwner(
			descriptor,
			&raw mut owner,
			&raw mut defaulted,
		))?;
		check(GetSecurityDescriptorDacl(
			descriptor,
			&raw mut present,
			&raw mut dacl,
			&raw mut defaulted,
		))?;
		check(GetSecurityDescriptorControl(
			descriptor,
			&raw mut control,
			&raw mut revision,
		))?;
		anyhow::ensure!(
			!owner.is_null()
				&& EqualSid(owner, user) != 0
				&& control & SE_DACL_PROTECTED != 0
				&& present != 0
				&& !dacl.is_null()
				&& (*dacl).AceCount == 1,
			"key file must have a protected owner-only DACL"
		);
		let mut ace = ptr::null_mut();
		check(GetAce(dacl, 0, &raw mut ace))?;
		let header = &*ace.cast::<ACE_HEADER>();
		anyhow::ensure!(
			header.AceType == 0
				&& header.AceFlags == 0
				&& usize::from(header.AceSize) >= size_of::<ACCESS_ALLOWED_ACE>(),
			"key file must contain an explicit allow ACE"
		);
		let ace = &*ace.cast::<ACCESS_ALLOWED_ACE>();
		anyhow::ensure!(
			ace.Mask == FILE_ALL_ACCESS
				&& EqualSid(ptr::addr_of!(ace.SidStart).cast_mut().cast(), user) != 0,
			"key file must grant access only to its owner"
		);
	}

	Ok(())
}

/// Preserves the OS error from a failed Windows call.
fn check(success: i32) -> io::Result<()> {
	if success == 0 {
		return Err(io::Error::last_os_error());
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn rejects_missing_inherited_and_non_owner_access() {
		// Synthetic descriptors let each unsafe-filesystem response be checked
		// without mounting a volume or requiring administrator privileges.
		unsafe {
			let mut user = ptr::null_mut();
			let sid: Vec<u16> = "S-1-5-18".encode_utf16().chain([0]).collect();
			check(ConvertStringSidToSidW(sid.as_ptr(), &raw mut user)).expect("user SID");
			let user = LocalMemory(user);

			for (sddl, valid) in [
				("O:SYD:P(A;;FA;;;SY)", true),
				("O:SY", false),
				("O:SYD:NO_ACCESS_CONTROL", false),
				("O:SYD:P", false),
				("O:SYD:(A;;FA;;;SY)", false),
				("O:BAD:P(A;;FA;;;SY)", false),
				("O:SYD:P(A;;FA;;;WD)", false),
				("O:SYD:P(A;;FA;;;SY)(A;;FR;;;WD)", false),
				("O:SYD:P(A;IO;FA;;;SY)", false),
				("O:SYD:P(D;;FA;;;SY)", false),
				("O:SYD:P(A;;FR;;;SY)", false),
			] {
				let text: Vec<u16> = sddl.encode_utf16().chain([0]).collect();
				let mut descriptor = ptr::null_mut();
				check(ConvertStringSecurityDescriptorToSecurityDescriptorW(
					text.as_ptr(),
					SDDL_REVISION_1,
					&raw mut descriptor,
					ptr::null_mut(),
				))
				.expect("test descriptor");
				let descriptor = LocalMemory(descriptor);
				assert_eq!(
					verify_descriptor(descriptor.0, user.0).is_ok(),
					valid,
					"{sddl}"
				);
			}
		}
	}
}
