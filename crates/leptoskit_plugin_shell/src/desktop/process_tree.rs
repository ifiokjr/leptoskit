use std::io;
use std::process::ExitStatus;
use std::time::Duration;

use process_wrap::tokio::ChildWrapper;
use process_wrap::tokio::CommandWrap;
use process_wrap::tokio::KillOnDrop;
use tokio::process::Command;
use tokio::sync::OwnedSemaphorePermit;

#[cfg(windows)]
mod windows_job;

/// Owns the OS container and its capacity until cleanup has completed.
pub(super) struct ProcessTree {
	child: Box<dyn ChildWrapper>,
	permit: Option<OwnedSemaphorePermit>,
	armed: bool,
	completed: Option<ExitStatus>,
	#[cfg(unix)]
	group: rustix::process::Pid,
}

impl ProcessTree {
	pub(super) fn spawn(command: Command, permit: OwnedSemaphorePermit) -> io::Result<Self> {
		let mut command = CommandWrap::from(command);
		command.wrap(KillOnDrop);
		#[cfg(unix)]
		command.wrap(process_wrap::tokio::ProcessGroup::leader());
		// Suspend until assignment, then retain process-handle witnesses until
		// every job member's termination is proven. Closing the job kills it.
		#[cfg(windows)]
		command.wrap(windows_job::JobObject);
		let child = command.spawn()?;

		Ok(Self {
			#[cfg(unix)]
			group: rustix::process::Pid::from_raw(
				i32::try_from(child.id().expect("new child has a PID")).expect("PID fits i32"),
			)
			.expect("new child has a positive PID"),
			child,
			permit: Some(permit),
			armed: true,
			completed: None,
		})
	}

	pub(super) fn child(&mut self) -> &mut dyn ChildWrapper {
		self.child.as_mut()
	}

	/// Observe the leader without releasing its PID before group termination.
	#[cfg(unix)]
	pub(super) async fn exited(&mut self) -> io::Result<()> {
		use rustix::process::WaitId;
		use rustix::process::WaitIdOptions;
		use tokio::signal::unix::SignalKind;
		use tokio::signal::unix::signal;

		// Subscribe before checking the status so an exit cannot be missed
		// between the non-reaping query and the asynchronous notification.
		let mut notifications = signal(SignalKind::child())?;
		let options = WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT;

		loop {
			match rustix::process::waitid(WaitId::Pid(self.group), options) {
				Ok(Some(_)) => return Ok(()),
				Ok(None) => {}
				Err(rustix::io::Errno::INTR) => continue,
				Err(error) => return Err(error.into()),
			}

			notifications.recv().await.ok_or_else(|| {
				io::Error::new(
					io::ErrorKind::BrokenPipe,
					"shell child-exit signal stream closed",
				)
			})?;
		}
	}

	#[cfg(windows)]
	pub(super) async fn exited(&mut self) -> io::Result<()> {
		windows_job::exited(self.child.as_mut()).await
	}

	pub(super) async fn stop(&mut self) -> io::Result<ExitStatus> {
		if let Some(status) = self.completed {
			return Ok(status);
		}

		// Always terminate the container, including after a normal leader exit.
		// A leader's status says nothing about descendants or inherited pipes.
		let termination_error: Option<io::Error> = match self.child.start_kill() {
			Ok(()) => None,
			// XNU can reject an exiting/all-zombie group before waitid reports
			// the leader as exited. Accept this only if cleanup proves
			// emptiness.
			#[cfg(target_os = "macos")]
			Err(error) if error.raw_os_error() == Some(rustix::io::Errno::PERM.raw_os_error()) => {
				Some(error)
			}
			Err(error) => return Err(error),
		};

		// Never signal this PGID again after releasing the leader's PID.
		self.armed = false;
		let cleanup = async {
			// Reap only our leader by its owned handle. A numeric group wait
			// could consume an unrelated child after the PGID is reused.
			#[cfg(unix)]
			let status = self.child.inner_mut().wait().await?;
			#[cfg(windows)]
			let status = self.child.wait().await?;

			#[cfg(unix)]
			loop {
				match rustix::process::test_kill_process_group(self.group) {
					Err(rustix::io::Errno::SRCH) => break,
					Ok(()) | Err(rustix::io::Errno::INTR | rustix::io::Errno::PERM) => {}
					Err(error) => return Err(io::Error::from(error)),
				}

				// Orphans are reaped by the OS. Their group must disappear
				// before admission reopens, even if the leader has already been
				// reaped.
				tokio::time::sleep(Duration::from_millis(5)).await;
			}

			Ok::<ExitStatus, io::Error>(status)
		};
		let status = tokio::time::timeout(Duration::from_secs(5), cleanup)
			.await
			.map_err(|_| {
				termination_error.unwrap_or_else(|| {
					io::Error::new(
						io::ErrorKind::TimedOut,
						"shell process container cleanup timed out",
					)
				})
			})??;

		drop(self.permit.take());
		self.completed = Some(status);
		Ok(status)
	}
}

impl Drop for ProcessTree {
	fn drop(&mut self) {
		if let Some(permit) = self.permit.take() {
			// Runtime shutdown or an I/O failure must still kill descendants.
			// Drop cannot join cleanup, so never advertise this slot as free.
			if self.armed {
				let _ = self.child.start_kill();
			}

			permit.forget();
		}
	}
}
