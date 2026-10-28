import 'package:arena_flutter/src/report.dart';
import 'package:arena_flutter/src/scenario.dart';
import 'package:arena_flutter/src/stats.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('reportLines', () {
    test('keeps a short report on one ARENA line', () {
      expect(reportLines('{"ok":true}'), ['ARENA {"ok":true}']);
    });

    test('splits a long report into numbered parts of at most 800 bytes', () {
      final json = '{"pad":"${'x' * 1900}"}';
      final lines = reportLines(json);

      expect(lines, hasLength(3));
      final chunks = [
        for (final (index, line) in lines.indexed)
          line.substring('ARENA-PART ${index + 1}/3 '.length),
      ];
      expect(lines.first, startsWith('ARENA-PART 1/3 {"pad"'));
      expect(chunks.every((chunk) => chunk.length <= maxReportChunk), isTrue);
      expect(chunks.join(), json);
    });

    test('rejects non-ASCII JSON, which byte-sized chunks could split', () {
      expect(() => reportLines('{"x":"é"}'), throwsArgumentError);
    });
  });

  group('Scenario.fromArguments', () {
    test('reads both argument forms', () {
      expect(Scenario.fromArguments(['--scenario', 'echo']), Scenario.echo);
      expect(Scenario.fromArguments(['--scenario=busy']), Scenario.busy);
      expect(Scenario.fromArguments(const []), isNull);
    });

    test('rejects an unknown scenario', () {
      expect(
        () => Scenario.fromArguments(['--scenario', 'nope']),
        throwsArgumentError,
      );
    });
  });

  group('stats', () {
    test('nearest-rank percentiles and medians', () {
      final values = [for (var i = 1; i <= 100; i += 1) i.toDouble()];
      expect(percentile(values, 50), 50);
      expect(percentile(values, 99), 99);
      expect(median([3, 1, 2, 4]), 2.5);
    });

    test('batch means', () {
      expect(batchMeans([1, 3, 5, 7], 2), [2, 6]);
      expect(() => batchMeans([1, 2, 3], 2), throwsArgumentError);
    });
  });
}
