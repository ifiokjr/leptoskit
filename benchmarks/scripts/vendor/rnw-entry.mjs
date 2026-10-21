// Explicit named re-exports (React Native's packages are CommonJS): the
// react-native-web surface plus React itself, one bundled React instance.
import * as React from "react";
import * as ReactNativeWeb from "react-native-web";

export const {
	createElement,
	useCallback,
	useMemo,
	useRef,
	useState,
	version,
} = React;
export const {
	AppRegistry,
	FlatList,
	Image,
	Pressable,
	ScrollView,
	StyleSheet,
	Text,
	View,
} = ReactNativeWeb;
