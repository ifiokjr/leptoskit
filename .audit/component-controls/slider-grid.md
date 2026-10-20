# Slider grid numerical contract

The rejected premise was: because every integer through 2^53 is representable, any floating-point slider with at most 2^53 intervals has a stable mapping between `min + index * step` and `round((value - min) / step)`.

That premise ignores the precision of the value and the subtraction at large magnitudes. The smallest spacing at `min` also says nothing about spacing near `max`. The rerunnable accepted-grid distribution test classifies failures by step spacing at the largest magnitude in the complete range before choosing a new domain boundary. This is a round-trip correctness experiment, not a timing benchmark.

The pre-fix run classified 200,000 generated configurations over 500 decimal exponents, plus both explicit reviewer cases. For step/endpoint-ULP bins `<1, <2, <4, <8, <16, >=16`, accepted counts were `2486, 3617, 3775, 3621, 3771, 182509` and reconstruction/drift failures were `85, 136, 26, 3, 0, 0`. Integer representability did not predict the failures; spacing across the complete value range did.

The domain now requires `step >= 16 * ULP(max(abs(min), abs(max)))`. Let that largest endpoint magnitude be M and its largest adjacent spacing U. The span is at most 2M, so this separation bounds the step index below 2^50. FMA contributes at most U/2 value error; subtracting min contributes at most U because its result has magnitude at most 2M. The permitted endpoint clamp can add at most another U. Dividing those errors by a step of at least 16U gives at most 2.5/16 index error. Correct rounding of an index below 2^50 adds at most 1/8, totaling 9/32, strictly below the half-index rounding boundary. This conservative bound therefore leaves headroom for reconstruction and an idempotent canonical step. It is derived from floating-point arithmetic, not a smaller arbitrary item-count cap. Subnormal grids use their real adjacent spacing as well. Near f64::MAX, the finite spacing below MAX is used.

The one-ULP value-space tolerance and FMA canonicalization remain unchanged. The last index is the nearest candidate if its value fits, otherwise that candidate minus one; using floor of an already rounded quotient can otherwise retain an out-of-range candidate.

Rerun the non-vacuous domain gate with:

`cargo test -p leptoskit_native --features serde --test controls accepted_slider_grids_preserve -- --nocapture`

The regression also checks the exact unsafe reviewer cases are rejected, the 16-ULP boundary, normal decimal sliders remain supported, and accepted large, negative, subnormal, and extreme finite grids preserve their proposals across construction, repeated quantization, and JSON serialization.

After the derived separation guard, the same 200,002 cases accept 182,509 configurations, all in the >=16-ULP bin, with zero reconstruction or drift failures. The gate asserts that more than 100,000 cases remain accepted, so a validator rejecting everything cannot pass. The explicit unsafe cases and just-below/exactly-at the 16-ULP boundary have separate assertions.

Independent read-only review then exercised two 3,000,000-configuration sweeps: 2,736,789 accepted at normal scales and 2,737,004 accepted across 600 decimal exponents, both without rejection or drift. The controls owner reran those compiled probes and observed the same results. A separate binary-exponent boundary/cancellation probe covered exponents -1070 through 1023: 25,126 accepted exactly-at-16-ULP configurations and 1,809,072 proposals, all stable. That binary-exponent probe is also encoded as a permanent integration test.
