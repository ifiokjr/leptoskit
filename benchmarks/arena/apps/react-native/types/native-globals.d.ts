// Runtime globals that Hermes and the browser both provide but that the
// React Native type package does not declare.

declare function requestAnimationFrame(
	callback: (timestamp: number) => void,
): number;
declare function cancelAnimationFrame(handle: number): void;
declare function setInterval(callback: () => void, ms: number): number;
declare function clearInterval(handle: number): void;
declare function setTimeout(callback: () => void, ms: number): number;
declare function btoa(data: string): string;
declare function atob(data: string): string;

declare const performance: { now(): number };

declare const console: { log(...values: unknown[]): void };
