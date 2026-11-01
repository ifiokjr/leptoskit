#![allow(unsafe_code)]

use std::collections::BTreeMap;
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
use windows_sys::Win32::Foundation::ERROR_INVALID_PARAMETER;
use windows_sys::Win32::Foundation::ERROR_NO_MORE_FILES;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::Foundation::WAIT_FAILED;
use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
use windows_sys::Win32::Foundation::WAIT_TIMEOUT;
use windows_sys::Win32::System::Diagnostics::ToolHelp::CreateToolhelp32Snapshot;
use windows_sys::Win32::System::Diagnostics::ToolHelp::TH32CS_SNAPTHREAD;
use windows_sys::Win32::System::Diagnostics::ToolHelp::THREADENTRY32;
use windows_sys::Win32::System::Diagnostics::ToolHelp::Thread32First;
use windows_sys::Win32::System::Diagnostics::ToolHelp::Thread32Next;
use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
use windows_sys::Win32::System::JobObjects::CreateJobObjectW;
use windows_sys::Win32::System::JobObjects::IsProcessInJob;
use windows_sys::Win32::System::JobObjects::JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
use windows_sys::Win32::System::JobObjects::JOBOBJECT_BASIC_ACCOUNTING_INFORMATION;
use windows_sys::Win32::System::JobObjects::JOBOBJECT_BASIC_PROCESS_ID_LIST;
use windows_sys::Win32::System::JobObjects::JOBOBJECT_EXTENDED_LIMIT_INFORMATION;
use windows_sys::Win32::System::JobObjects::JobObjectBasicAccountingInformation;
use windows_sys::Win32::System::JobObjects::JobObjectBasicProcessIdList;
use windows_sys::Win32::System::JobObjects::JobObjectExtendedLimitInformation;
use windows_sys::Win32::System::JobObjects::QueryInformationJobObject;
use windows_sys::Win32::System::JobObjects::SetInformationJobObject;
use windows_sys::Win32::System::JobObjects::TerminateJobObject;
use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;
use windows_sys::Win32::System::Threading::OpenProcess;
use windows_sys::Win32::System::Threading::OpenThread;
use windows_sys::Win32::System::Threading::PROCESS_QUERY_LIMITED_INFORMATION;
use windows_sys::Win32::System::Threading::PROCESS_SYNCHRONIZE;
use windows_sys::Win32::System::Threading::ResumeThread;
use windows_sys::Win32::System::Threading::THREAD_SUSPEND_RESUME;
use windows_sys::Win32::System::Threading::WaitForSingleObject;

const MAX_TRACKED: usize = 4096;

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
		let processes = BTreeMap::from([(pid, process.try_clone_to_owned()?)]);
		let snapshot = vec![0; MAX_TRACKED + 2];
		resume(pid)?;
		Ok(Box::new(JobChild {
			child,
			job,
			processes,
			snapshot,
		}))
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
				u32::try_from(size_of_val(&limits)).expect("job limits fit DWORD"),
			)
		})?;
		Ok(Self(job))
	}

	fn total_processes(&self) -> io::Result<u32> {
		let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
		// SAFETY: the writable buffer matches the information class and size;
		// the owned job handle is open and the optional length output is
		// unused.
		check(unsafe {
			QueryInformationJobObject(
				self.0.as_raw_handle(),
				JobObjectBasicAccountingInformation,
				std::ptr::from_mut(&mut accounting).cast(),
				u32::try_from(size_of_val(&accounting)).expect("job accounting fits DWORD"),
				std::ptr::null_mut(),
			)
		})?;
		Ok(accounting.TotalProcesses)
	}

	fn members<'a>(&self, buffer: &'a mut [usize]) -> io::Result<&'a [usize]> {
		let header = buffer
			.as_mut_ptr()
			.cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>();
		// SAFETY: usize alignment meets the header's alignment. The buffer
		// includes the header and MAX_TRACKED IDs, and its exact byte size is
		// passed to the OS. Partial or oversized snapshots are rejected.
		check(unsafe {
			QueryInformationJobObject(
				self.0.as_raw_handle(),
				JobObjectBasicProcessIdList,
				header.cast(),
				u32::try_from(size_of_val(buffer)).expect("process snapshot fits DWORD"),
				std::ptr::null_mut(),
			)
		})?;
		// SAFETY: a successful query initialized this header within buffer.
		let header = unsafe { &*header };
		let count = usize::try_from(header.NumberOfProcessIdsInList).expect("DWORD fits usize");

		if header.NumberOfAssignedProcesses != header.NumberOfProcessIdsInList
			|| count > MAX_TRACKED
		{
			return Err(io::Error::other("incomplete shell job process snapshot"));
		}
		let offset = std::mem::offset_of!(JOBOBJECT_BASIC_PROCESS_ID_LIST, ProcessIdList)
			/ size_of::<usize>();
		buffer
			.get(offset..offset + count)
			.ok_or_else(|| io::Error::other("invalid shell job process snapshot"))
	}

	fn contains(&self, process: &OwnedHandle) -> io::Result<bool> {
		let mut member = 0;
		// SAFETY: both owned handles remain open and member is a writable BOOL.
		check(unsafe {
			IsProcessInJob(
				process.as_raw_handle(),
				self.0.as_raw_handle(),
				&raw mut member,
			)
		})?;
		Ok(member != 0)
	}
}

#[derive(Debug)]
struct JobChild {
	child: Box<dyn ChildWrapper>,
	job: Job,
	processes: BTreeMap<u32, OwnedHandle>,
	snapshot: Vec<usize>,
}

impl JobChild {
	fn observe(&mut self) -> io::Result<()> {
		for pid in self.job.members(&mut self.snapshot)? {
			let pid = u32::try_from(*pid).map_err(io::Error::other)?;

			if self.processes.contains_key(&pid) {
				continue;
			}

			if self.processes.len() == MAX_TRACKED {
				return Err(io::Error::other("shell process tracking limit exceeded"));
			}

			// SAFETY: this opens only a query/synchronization handle. A PID
			// obtained from the job may already have exited or been reused;
			// membership is checked before counting it as a witness.
			let process = match owned(unsafe {
				OpenProcess(
					PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
					0,
					pid,
				)
			}) {
				Ok(process) => process,

				Err(error) if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) => {
					continue;
				}
				Err(error) => return Err(error),
			};

			if self.job.contains(&process)? {
				self.processes.insert(pid, process);
			}

			// A lost process identity is never counted. The lifetime process
			// total below makes any missing witness fail closed.
		}

		Ok(())
	}
}

pub(super) async fn exited(child: &mut dyn ChildWrapper) -> io::Result<()> {
	let child = (child as &mut dyn std::any::Any)
		.downcast_mut::<JobChild>()
		.expect("Windows shell child owns a job");

	loop {
		child.observe()?;

		if child.child.try_wait()?.is_some() {
			return Ok(());
		}

		tokio::time::sleep(Duration::from_millis(5)).await;
	}
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
		let observed = self.observe();
		// SAFETY: this handle identifies only the invocation's owned job.
		check(unsafe { TerminateJobObject(self.job.0.as_raw_handle(), 1) })?;
		observed
	}

	fn wait(&mut self) -> Pin<Box<dyn Future<Output = io::Result<ExitStatus>> + Send + '_>> {
		Box::pin(async {
			let status = self.child.wait().await?;

			loop {
				self.observe()?;
				let mut complete = true;

				for process in self.processes.values() {
					// SAFETY: every witness is an owned process handle opened
					// for synchronization, and a zero timeout cannot block.
					match unsafe { WaitForSingleObject(process.as_raw_handle(), 0) } {
						WAIT_OBJECT_0 => {}
						WAIT_TIMEOUT => complete = false,

						WAIT_FAILED => return Err(io::Error::last_os_error()),
						_ => return Err(io::Error::other("unexpected shell process wait status")),
					}
				}

				if complete {
					// Handles pin unique identities. Their signaled state
					// proves termination; matching the monotonic lifetime total
					// proves that no job member was missed. Accounting zero
					// alone does not provide this guarantee.
					let tracked =
						u32::try_from(self.processes.len()).expect("bounded witness count");

					if self.job.total_processes()? != tracked {
						return Err(io::Error::other(
							"shell cleanup could not witness every job member",
						));
					}

					return Ok(status);
				}

				tokio::time::sleep(Duration::from_millis(5)).await;
			}
		})
	}
}

fn resume(pid: u32) -> io::Result<()> {
	// SAFETY: the returned snapshot is validated and owned by this function.
	let snapshot = owned(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) })?;
	let mut entry = THREADENTRY32 {
		dwSize: u32::try_from(size_of::<THREADENTRY32>()).expect("thread entry fits DWORD"),
		..Default::default()
	};
	// SAFETY: the snapshot is open and entry declares its exact buffer size.
	check(unsafe { Thread32First(snapshot.as_raw_handle(), &raw mut entry) })?;
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
		if unsafe { Thread32Next(snapshot.as_raw_handle(), &raw mut entry) } == 0 {
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

#[cfg(test)]
mod tests {
	use super::*;

	#[tokio::test]
	async fn an_unobserved_member_cannot_be_reported_as_clean() {
		let system_root = std::env::var_os("SystemRoot").expect("Windows has SystemRoot");
		let executable = std::path::Path::new(&system_root)
			.join("System32")
			.join("cmd.exe");
		let mut command = Command::new(&executable);
		command
			.env_clear()
			.env("SystemRoot", &system_root)
			.args(["/d", "/c"])
			.arg(&executable)
			.args(["/d", "/c", "exit", "0"]);
		let mut command = CommandWrap::from(command);
		command.wrap(process_wrap::tokio::KillOnDrop);
		command.wrap(JobObject);
		let mut child = command.spawn().unwrap();

		let root = usize::try_from(child.id().unwrap()).unwrap();
		let child = (child.as_mut() as &mut dyn std::any::Any)
			.downcast_mut::<JobChild>()
			.unwrap();

		// Model a supervisor that was not scheduled until the root and its
		// short-lived cmd child finished. No fixture identities are altered.
		assert!(
			tokio::time::timeout(Duration::from_secs(5), child.child.wait())
				.await
				.unwrap()
				.unwrap()
				.success()
		);
		tokio::time::timeout(Duration::from_secs(5), async {
			while child
				.job
				.members(&mut child.snapshot)
				.unwrap()
				.iter()
				.any(|pid| *pid != root)
			{
				tokio::time::sleep(Duration::from_millis(5)).await;
			}
		})
		.await
		.unwrap();
		assert!(child.job.total_processes().unwrap() > 1);
		child.start_kill().unwrap();
		let error = tokio::time::timeout(Duration::from_secs(5), child.wait())
			.await
			.unwrap()
			.unwrap_err();
		assert_eq!(
			error.to_string(),
			"shell cleanup could not witness every job member"
		);
	}
}
