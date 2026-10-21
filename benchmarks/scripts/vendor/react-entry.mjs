// Explicit named re-exports: React is CommonJS, so `export *` cannot
// enumerate it. This is the exact surface the benchmark app imports.
import * as React from "react";
import * as ReactDOMClient from "react-dom/client";

export const {
	createElement,
	createContext,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
	version,
} = React;
export const { createRoot, hydrateRoot } = ReactDOMClient;
