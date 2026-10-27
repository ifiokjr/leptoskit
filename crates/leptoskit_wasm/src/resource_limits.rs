//! Fixed per-host budgets for initial allocations and subsequent guest growth.

use wasmi::ResourceLimiter;
use wasmi_core::LimiterError;

use crate::MAX_MEMORY_BYTES;
use crate::MAX_TABLE_ELEMENTS;

/// A fresh host always carries the same bounds, including unexported resources.
#[derive(Debug, Default)]
pub(super) struct ResourceLimits;

impl ResourceLimiter for ResourceLimits {
	fn memory_growing(
		&mut self,
		_current: usize,
		desired: usize,
		_maximum: Option<usize>,
	) -> Result<bool, LimiterError> {
		if desired > MAX_MEMORY_BYTES {
			return Err(LimiterError::ResourceLimiterDeniedAllocation);
		}

		Ok(true)
	}

	fn table_growing(
		&mut self,
		_current: usize,
		desired: usize,
		_maximum: Option<usize>,
	) -> Result<bool, LimiterError> {
		if desired > MAX_TABLE_ELEMENTS {
			return Err(LimiterError::ResourceLimiterDeniedAllocation);
		}

		Ok(true)
	}

	// Keep the trait's default grow-failure callbacks: converting every failure
	// into a limiter trap would hide OutOfFuel and change guest-declared
	// maxima.
	fn instances(&self) -> usize {
		1
	}

	fn tables(&self) -> usize {
		2
	}

	fn memories(&self) -> usize {
		1
	}
}
