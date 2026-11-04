//! Small platform sampler with no background daemon or privileged access.
use crate::HostSample;

#[must_use]
pub fn sample() -> HostSample {
	let mut sample = HostSample {
		at_unix_seconds: std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap_or_default()
			.as_secs(),
		..HostSample::default()
	};
	#[cfg(target_os = "linux")]
	{
		sample.process_resident_bytes =
			std::fs::read_to_string("/proc/self/status")
				.ok()
				.and_then(|status| {
					status.lines().find_map(|line| {
						line.strip_prefix("VmRSS:").and_then(|v| {
							v.split_whitespace()
								.next()?
								.parse::<u64>()
								.ok()?
								.checked_mul(1024)
						})
					})
				});
		let cgroup = std::fs::read_to_string("/proc/self/cgroup")
			.ok()
			.and_then(|text| {
				text.lines()
					.find_map(|line| line.strip_prefix("0::").map(str::to_owned))
			});
		if let Some(relative) = cgroup {
			let root = std::path::Path::new("/sys/fs/cgroup");
			let resolved = root.join(relative.trim_start_matches('/'));
			// Container namespaces commonly mount their cgroup directly at
			// root.
			let root = if resolved.join("memory.current").exists() {
				resolved
			} else {
				root.to_path_buf()
			};
			sample.cgroup_memory_bytes = number(&root.join("memory.current"));
			sample.cgroup_memory_limit_bytes = number(&root.join("memory.max"));
			sample.cgroup_cpu_seconds = std::fs::read_to_string(root.join("cpu.stat"))
				.ok()
				.and_then(|text| {
					text.lines()
						.find_map(|line| line.strip_prefix("usage_usec ")?.parse::<f64>().ok())
				})
				.map(|micros| micros / 1_000_000.0);
			sample.cgroup_cpu_limit_cores = std::fs::read_to_string(root.join("cpu.max"))
				.ok()
				.and_then(|text| {
					let mut values = text.split_whitespace();
					let quota = values.next()?.parse::<f64>().ok()?;
					let period = values.next()?.parse::<f64>().ok()?;
					(period > 0.0).then_some(quota / period)
				});
		}
	}
	#[cfg(unix)]
	if let Ok(stats) = rustix::fs::statvfs("/") {
		sample.filesystem_available_bytes = stats.f_bavail.checked_mul(stats.f_frsize);
		sample.filesystem_capacity_bytes = stats.f_blocks.checked_mul(stats.f_frsize);
	}
	sample
}

#[cfg(target_os = "linux")]
fn number(path: &std::path::Path) -> Option<u64> {
	std::fs::read_to_string(path).ok()?.trim().parse().ok()
}
