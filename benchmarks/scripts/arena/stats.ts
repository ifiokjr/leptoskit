// Summary statistics for arena samples: the median, the 95th percentile,
// and a bootstrap 95% confidence interval for the median.

export interface Summary {
	values: number[];
	median: number;
	p95: number;
	min: number;
	max: number;
	/** The bootstrap 95% interval for the median, with at least five samples. */
	ci95?: [number, number];
}

function median(sorted: number[]): number {
	const middle = Math.floor(sorted.length / 2);
	return sorted.length % 2 === 1
		? sorted[middle]
		: (sorted[middle - 1] + sorted[middle]) / 2;
}

/** A fixed-seed generator, so a summary is reproducible from its samples. */
function random(seed: number): () => number {
	let state = seed >>> 0;
	return () => {
		state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
		return state / 4294967296;
	};
}

const RESAMPLES = 2000;

export function summarise(values: number[]): Summary {
	if (values.length === 0) throw new Error("cannot summarise no samples");
	const sorted = [...values].sort((left, right) => left - right);
	const summary: Summary = {
		values,
		median: median(sorted),
		p95:
			sorted[Math.min(sorted.length - 1, Math.ceil(0.95 * sorted.length) - 1)],
		min: sorted[0],
		max: sorted[sorted.length - 1],
	};
	if (values.length >= 5) {
		const next = random(values.length);
		const medians: number[] = [];
		for (let resample = 0; resample < RESAMPLES; resample += 1) {
			const draw = Array.from(
				{ length: values.length },
				() => values[Math.floor(next() * values.length)],
			);
			medians.push(median(draw.sort((left, right) => left - right)));
		}
		medians.sort((left, right) => left - right);
		summary.ci95 = [
			medians[Math.floor(RESAMPLES * 0.025)],
			medians[Math.floor(RESAMPLES * 0.975)],
		];
	}
	return summary;
}
