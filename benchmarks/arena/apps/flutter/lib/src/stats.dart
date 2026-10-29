/// The nearest-rank percentile of [values], for `percent` in 0–100.
double percentile(List<double> values, double percent) {
  if (values.isEmpty) throw ArgumentError('percentile of no values');
  final sorted = [...values]..sort();
  final rank = (percent / 100 * sorted.length).ceil().clamp(1, sorted.length);
  return sorted[rank - 1];
}

/// The median of [values], averaging the middle pair for an even count.
double median(List<double> values) {
  if (values.isEmpty) throw ArgumentError('median of no values');
  final sorted = [...values]..sort();
  final middle = sorted.length ~/ 2;
  return sorted.length.isOdd
      ? sorted[middle]
      : (sorted[middle - 1] + sorted[middle]) / 2;
}

/// The mean of each consecutive batch of [batch] values.
List<double> batchMeans(List<double> values, int batch) {
  if (values.length % batch != 0) {
    throw ArgumentError('${values.length} values do not split into $batch');
  }
  return [
    for (var start = 0; start < values.length; start += batch)
      values.sublist(start, start + batch).reduce((a, b) => a + b) / batch,
  ];
}

/// Rounds to three decimal places, enough for microsecond timings measured in
/// nanosecond ticks without printing noise digits.
double round3(double value) => (value * 1000).roundToDouble() / 1000;
