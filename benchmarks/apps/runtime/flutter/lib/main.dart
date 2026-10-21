// The tablebench Flutter web app: the same interface shape as the other
// fixtures — scenario buttons, an increment button, a scrolling list of
// rows with deterministic labels.
//
// Flutter renders to a canvas, so the shared DOM readers cannot verify this
// app's table (bench.app.json marks it `domScenarios: false`); it reports
// startup, transfer, and memory metrics, which is where canvas-rendered web
// apps most differ from DOM frameworks.
import 'dart:js_interop';

import 'package:flutter/material.dart';

@JS('__bench.labelFor')
external String _labelFor(int id);

@JS('__bench.mounted')
external void _benchMounted();

void main() {
	runApp(const TableBenchApp());
}

class TableBenchApp extends StatelessWidget {
	const TableBenchApp({super.key});

	@override
	Widget build(BuildContext context) {
		return const MaterialApp(
			title: 'tablebench',
			debugShowCheckedModeBanner: false,
			home: TableBench(),
		);
	}
}

class TableBench extends StatefulWidget {
	const TableBench({super.key});

	@override
	State<TableBench> createState() => _TableBenchState();
}

class _TableBenchState extends State<TableBench> {
	final List<({int id, String label})> _rows = [];
	int _nextId = 1;
	int _count = 0;
	int _selected = -1;

	List<({int id, String label})> _build(int total) {
		return List.generate(
			total,
			(int index) => (
				id: _nextId + index,
				label: _labelFor(_nextId + index),
			),
			growable: false,
		);
	}

	void _create(int total) {
		setState(() {
			_rows
				..clear()
				..addAll(_build(total));
			_nextId += total;
		});
	}

	@override
	Widget build(BuildContext context) {
		// Signal mount after the first frame so the startup metric measures
		// engine boot plus the first paint, not just script evaluation.
		WidgetsBinding.instance.addPostFrameCallback((_) => _benchMounted());
		return Scaffold(
			body: Column(
				children: [
					Wrap(
						children: [
							for (final (label, action) in [
								('create 1000', () => _create(1000)),
								('create 10000', () => _create(10000)),
								('append 1000', () => setState(() {
									_rows.addAll(_build(1000));
									_nextId += 1000;
								})),
								('update every 10th', () => setState(() {
									for (var index = 0; index < _rows.length; index += 10) {
										_rows[index] = (
											id: _rows[index].id,
											label: '${_rows[index].label} !!!',
										);
									}
								})),
								('swap rows', () => setState(() {
									if (_rows.length > 998) {
										final first = _rows[1];
										_rows[1] = _rows[998];
										_rows[998] = first;
									}
								})),
								('remove row', () => setState(() {
									if (_rows.length > 500) {
										_rows.removeAt(500);
									}
								})),
								('select row', () => setState(() {
									if (_rows.length > 42) {
										_selected = _rows[42].id;
									}
								})),
								('clear', () => setState(_rows.clear)),
							])
								TextButton(
									onPressed: action,
									child: Text(label),
								),
							TextButton(
								onPressed: () => setState(() => _count += 1),
								child: const Text('increment'),
							),
							Text('count: $_count'),
						],
					),
					Expanded(
						child: ListView.builder(
							itemCount: _rows.length,
							itemBuilder: (context, index) {
								final row = _rows[index];
								return Container(
									color: row.id == _selected ? Colors.amber : null,
									padding: const EdgeInsets.symmetric(horizontal: 8),
									child: Text(
										'${row.id}  ${row.label}',
										maxLines: 1,
									),
								);
							},
						),
					),
				],
			),
		);
	}
}
