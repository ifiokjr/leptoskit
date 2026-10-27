#![allow(unsafe_code)]

use std::future::Future;
use std::io;
use std::os::windows::io::AsRawHandle;
use std::os::windows::io::BorrowedHandle;
use std::os::windows::io::FromRawHandle;
use std::os::windows::io::OwnedHandle;
use std::pin::Pin;
use std::process::ExitStatus;
use std::time::Duration;

use process_wrap::tokio::ChildWrapper;
use process_wrap::tokio::CommandWrap;
use process_wrap::tokio::CommandWrapper;
use tokio::process::Command;
use windows_sys::Win32::Foundation::ERROR_NO_MORE_FILES;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::System::Diagnostics::ToolHelp::CreateToolhelp32Snapshot;
use windows_sys::Win32::System::Diagnostics::ToolHelp::TH32CS_SNAPTHREAD;
use windows_sys::Win32::System::Diagnostics::ToolHelp::THREADENTRY32;
use windows_sys::Win32::System::Diagnostics::ToolHelp::Thread32First;
use windows_sys::Win32::System::Diagnostics::ToolHelp::Thread32Next;
use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
use windows_sys::Win32::System::JobObjects::CreateJobObjectW;
use windows_sys::Win32::System::JobObjects::JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
use windows_sys::Win32::System::JobObjects::JOBOBJECT_BASIC_ACCOUNTING_INFORMATION;
use windows_sys::Win32::System::JobObjects::JOBOBJECT_EXTENDED_LIMIT_INFORMATION;
use windows_sys::Win32::System::JobObjects::JobObjectBasicAccountingInformation;
use windows_sys::Win32::System::JobObjects::JobObjectExtendedLimitInformation;
use windows_sys::Win32::System::JobObjects::QueryInformationJobObject;
use windows_sys::Win32::System::JobObjects::SetInformationJobObject;
use windows_sys::Win32::System::JobObjects::TerminateJobObject;
use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;
use windows_sys::Win32::System::Threading::OpenThread;
use windows_sys::Win32::System::Threading::ResumeThread;
use windows_sys::Win32::System::Threading::THREAD_SUSPEND_RESUME;

#[derive(Debug)]
pub(super) struct JobObject;

impl CommandWrapper for JobObject {
	fn pre_spawn(&mut self, command: &mut Command, _: &CommandWrap) -> io::Result<()> {
		command.creation_flags(CREATE_SUSPENDED);
		Ok(())
	}

	fn wrap_child(
		&mut self,
		child: Box<dyn ChildWrapper>,
		_: &CommandWrap,
	) -> io::Result<Box<dyn ChildWrapper>> {
		let job = Job::new()?;
		let pid = child.id().expect("new suspended child has a PID");
		let process = child.process_handle().expect("native child has a handle");
		// SAFETY: both handles remain owned and open through assignment. The
		// child's primary thread cannot run before resume below.
		check(unsafe { AssignProcessToJobObject(job.0.as_raw_handle(), process.as_raw_handle()) })?;
		resume(pid)?;
		Ok(Box::new(JobChild { child, job }))
	}
}

#[derive(Debug)]
struct Job(OwnedHandle);

impl Job {
	fn new() -> io::Result<Self> {
		// SAFETY: null attributes create a non-inheritable, unnamed owned job.
		let job = owned(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) })?;
		let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
		limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
		// SAFETY: this information class matches the initialized buffer and
		// exact size. The job handle stays open throughout the call.
		check(unsafe {
			SetInformationJobObject(
				job.as_raw_handle(),
				JobObjectExtendedLimitInformation,
				std::ptr::from_ref(&limits).cast(),
				u32::try_from(std::mem::size_of_val(&limits)).expect("job limits fit DWORD"),
			)
		})?;
		Ok(Self(job))
	}

	fn empty(&self) -> io::Result<bool> {
		let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
		// SAFETY: the writable buffer matches the information class and size;
		// the owned job handle is open and the optional length output is
		// unused.
		check(unsafe {
			QueryInformationJobObject(
				self.0.as_raw_handle(),
				JobObjectBasicAccountingInformation,
				std::ptr::from_mut(&mut accounting).cast(),
				u32::try_from(std::mem::size_of_val(&accounting))
					.expect("job accounting fits DWORD"),
				std::ptr::null_mut(),
			)
		})?;
		Ok(accounting.ActiveProcesses == 0)
	}
}

#[derive(Debug)]
struct JobChild {
	child: Box<dyn ChildWrapper>,
	job: Job,
}

impl ChildWrapper for JobChild {
	fn inner(&self) -> &dyn ChildWrapper {
		self.child.as_ref()
	}

	fn inner_mut(&mut self) -> &mut dyn ChildWrapper {
		self.child.as_mut()
	}

	fn into_inner(self: Box<Self>) -> Box<dyn ChildWrapper> {
		self.child
	}

	fn process_handle(&self) -> Option<BorrowedHandle<'_>> {
		self.child.process_handle()
	}

	fn start_kill(&mut self) -> io::Result<()> {
		// SAFETY: this handle identifies only the invocation's owned job.
		check(unsafe { TerminateJobObject(self.job.0.as_raw_handle(), 1) })
	}

	fn wait(&mut self) -> Pin<Box<dyn Future<Output = io::Result<ExitStatus>> + Send + '_>> {
		Box::pin(async {
			let status = self.child.wait().await?;
			// A leader exit or a completion-port notification alone does not
			// prove that every member has stopped. Keep capacity until zero.
			while !self.job.empty()? {
				tokio::time::sleep(Duration::from_millis(5)).await;
			}
			Ok(status)
		})
	}
}

fn resume(pid: u32) -> io::Result<()> {
	// SAFETY: the returned snapshot is validated and owned by this function.
	let snapshot = owned(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) })?;
	let mut entry = THREADENTRY32 {
		dwSize: u32::try_from(std::mem::size_of::<THREADENTRY32>())
			.expect("thread entry fits DWORD"),
		..Default::default()
	};
	// SAFETY: the snapshot is open and entry declares its exact buffer size.
	check(unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) })?;
	let mut resumed = false;
	loop {
		if entry.th32OwnerProcessID == pid {
			// SAFETY: the suspended owned child still holds this PID; only
			// threads belonging to that process are opened for resumption.
			let thread =
				owned(unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) })?;
			// SAFETY: the owned thread handle is valid until after this call.
			let count = unsafe { ResumeThread(thread.as_raw_handle()) };
			if count == u32::MAX {
				return Err(io::Error::last_os_error());
			}
			resumed |= count > 0;
		}
		// SAFETY: both the snapshot and initialized entry remain valid.
		if unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) } == 0 {
			let error = io::Error::last_os_error();
			if error.raw_os_error() != Some(ERROR_NO_MORE_FILES as i32) {
				return Err(error);
			}
			break;
		}
	}
	if resumed {
		Ok(())
	} else {
		Err(io::Error::other(
			"suspended shell child's thread was not found",
		))
	}
}

fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
	if handle.is_null() || handle == INVALID_HANDLE_VALUE {
		return Err(io::Error::last_os_error());
	}
	// SAFETY: callers pass newly acquired, uniquely owned kernel handles.
	Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

fn check(result: i32) -> io::Result<()> {
	if result == 0 {
		Err(io::Error::last_os_error())
	} else {
		Ok(())
	}
}
