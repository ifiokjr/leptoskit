import { useCallback, useState } from "@lynx-js/react";

import "./App.css";
import {
	emitReport,
	type Phase,
	PHASE_COLOURS,
	readLaunch,
	type ScenarioId,
	type ScenarioResult,
	SCENARIOS,
} from "./arena.js";
import { Cold } from "./scenarios/Cold.js";
import { Echo } from "./scenarios/Echo.js";
import { Autoscroll, Busy } from "./scenarios/Feed.js";
import { Mount } from "./scenarios/Mount.js";
import { Ticker } from "./scenarios/Ticker.js";
import { TickerTuned } from "./scenarios/TickerTuned.js";

/** What every scenario component receives from the app shell. */
export interface ScenarioProps {
	setPhase(phase: Phase): void;
	finish(result: ScenarioResult): void;
}

const launch = readLaunch();

export function App() {
	const [scenario, setScenario] = useState<ScenarioId | undefined>(
		launch.scenario,
	);
	// `cold` is done in its very first frame: the ready screen is the first
	// thing the main thread renders.
	const [phase, setPhaseState] = useState<Phase>(
		launch.scenario === "cold" ? "done" : "idle",
	);

	const setPhase = useCallback((next: Phase) => {
		"background only";
		setPhaseState(next);
	}, []);

	const finish = useCallback((result: ScenarioResult) => {
		"background only";
		if (scenario === undefined) return;
		const lane = scenario === "ticker" ? launch.lane : "idiomatic";
		emitReport({
			v: 1,
			framework: "lynx",
			frameworkVersion: launch.frameworkVersion,
			platform: launch.platform,
			scenario,
			lane,
			ok: result.ok,
			...(result.error === undefined ? {} : { error: result.error }),
			...(result.notes === undefined || result.notes.length === 0
				? {}
				: { notes: result.notes }),
			metrics: result.metrics,
			...(result.samples === undefined ? {} : { samples: result.samples }),
		});
		setPhaseState(result.ok ? "done" : "failed");
	}, [scenario]);

	const start = useCallback((next: ScenarioId) => {
		"background only";
		setPhaseState(next === "cold" ? "done" : "idle");
		setScenario(next);
	}, []);

	return (
		<view className="root">
			<view className="header">
				<view
					className="beacon"
					style={{ backgroundColor: PHASE_COLOURS[phase] }}
				/>
				<text className="status">
					{scenario === undefined ? "home" : `${scenario} ${phase}`}
				</text>
			</view>
			<view className="body">
				{scenario === undefined ? <Home start={start} /> : (
					<ScenarioView
						scenario={scenario}
						setPhase={setPhase}
						finish={finish}
					/>
				)}
			</view>
		</view>
	);
}

function ScenarioView(props: ScenarioProps & { scenario: ScenarioId }) {
	const { scenario, setPhase, finish } = props;
	switch (scenario) {
		case "cold":
			return <Cold setPhase={setPhase} finish={finish} />;
		case "mount":
			return <Mount setPhase={setPhase} finish={finish} />;
		case "echo":
			return <Echo setPhase={setPhase} finish={finish} />;
		case "ticker":
			return launch.lane === "tuned"
				? <TickerTuned setPhase={setPhase} finish={finish} />
				: <Ticker setPhase={setPhase} finish={finish} />;
		case "autoscroll":
			return <Autoscroll setPhase={setPhase} finish={finish} />;
		case "busy":
			return <Busy setPhase={setPhase} finish={finish} />;
	}
}

function Home(props: { start(scenario: ScenarioId): void }) {
	return (
		<scroll-view className="fill" scroll-orientation="vertical">
			{SCENARIOS.map((scenario) => (
				<view
					key={scenario}
					className="button"
					bindtap={() => props.start(scenario)}
				>
					<text className="button-label">{scenario}</text>
				</view>
			))}
		</scroll-view>
	);
}
