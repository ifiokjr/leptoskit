//! Xcode project assembly for the generated native application.
use super::*;
pub(super) fn generate(
	root: &Path,
	config: &leptoskit_config::LeptoskitConfig,
	registry: &Registry,
) -> Result<()> {
	let dir = root.join(".leptoskit/ios");
	// Cargo registry directories have version suffixes. Give Swift packages
	// stable sibling names so their local bridge dependency also resolves
	// after publication, without modifying Cargo's cached sources.
	let package_root = dir.join("packages");
	let bridge_package = package_root.join("leptoskit_bridge_ffi");
	copy_package(&registry.bridge, &bridge_package)?;
	write(
		&dir.join("GeneratedPlugins.swift"),
		&ios_registrant(registry)?,
	)?;
	let mut objects = String::new();
	let (source_files, source_refs) = ios_sources(root, registry, &dir, &mut objects)?;
	let (package_refs, products, frameworks) =
		package_references(registry, &package_root, bridge_package, &mut objects)?;
	// The store-shipped guest travels only inside its container; the host reads
	// its Wasm through the update store.
	let mobile = super::super::generated_dir(root, Target::Ios).join("mobile");

	for (offset, name) in [(BUNDLE_FILE, "app.lkb"), (UPDATE_FILE, "codepush.json")] {
		writeln!(
			objects,
			"{} = {{isa = PBXFileReference; lastKnownFileType = file; path = {}; sourceTree = \
			 \"<absolute>\"; }};\n{} = {{isa = PBXBuildFile; fileRef = {}; }};",
			id(offset),
			quote(&mobile.join(name))?,
			id(offset + 1),
			id(offset)
		)?;
	}

	build_phases(
		&mut objects,
		root,
		config,
		&source_refs,
		&source_files,
		&frameworks,
	)?;
	ios_configurations(&mut objects, root, config, &package_root)?;
	targets(&mut objects, &products, &package_refs)?;
	write(
		&dir.join("Host.xcodeproj/project.pbxproj"),
		&format!(
			"// !$*UTF8*$!\n{{ archiveVersion = 1; classes = {{}}; objectVersion = 56; objects = \
			 {{\n{objects}}}; rootObject = {}; }}\n",
			id(PROJECT)
		),
	)?;
	let scheme = format!(
		r#"<?xml version="1.0" encoding="UTF-8"?>
<Scheme LastUpgradeVersion="1600" version="1.3">
<BuildAction parallelizeBuildables="YES" buildImplicitDependencies="YES"><BuildActionEntries><BuildActionEntry buildForTesting="YES" buildForRunning="YES" buildForProfiling="YES" buildForArchiving="YES" buildForAnalyzing="YES"><BuildableReference BuildableIdentifier="primary" BlueprintIdentifier="{}" BuildableName="App.app" BlueprintName="App" ReferencedContainer="container:Host.xcodeproj"/></BuildActionEntry></BuildActionEntries></BuildAction>
<TestAction buildConfiguration="Debug" selectedDebuggerIdentifier="Xcode.DebuggerFoundation.Debugger.LLDB" selectedLauncherIdentifier="Xcode.IDEFoundation.Launcher.LLDB"><Testables><TestableReference skipped="NO"><BuildableReference BuildableIdentifier="primary" BlueprintIdentifier="{}" BuildableName="AppUITests.xctest" BlueprintName="AppUITests" ReferencedContainer="container:Host.xcodeproj"/></TestableReference></Testables></TestAction>
<LaunchAction buildConfiguration="Debug" selectedDebuggerIdentifier="Xcode.DebuggerFoundation.Debugger.LLDB" selectedLauncherIdentifier="Xcode.IDEFoundation.Launcher.LLDB" launchStyle="0" useCustomWorkingDirectory="NO" ignoresPersistentStateOnLaunch="NO" debugDocumentVersioning="YES" allowLocationSimulation="YES"><BuildableProductRunnable runnableDebuggingMode="0"><BuildableReference BuildableIdentifier="primary" BlueprintIdentifier="{}" BuildableName="App.app" BlueprintName="App" ReferencedContainer="container:Host.xcodeproj"/></BuildableProductRunnable></LaunchAction>
<ArchiveAction buildConfiguration="Release" revealArchiveInOrganizer="YES"/>
</Scheme>"#,
		id(APP_TARGET),
		id(TEST_TARGET),
		id(APP_TARGET)
	);
	write(
		&dir.join("Host.xcodeproj/xcshareddata/xcschemes/App.xcscheme"),
		&scheme,
	)
}
fn ios_tests(objects: &mut String, root: &Path, package: &str) -> Result<()> {
	writeln!(
		objects,
		"{} = {{isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = {}; \
		 sourceTree = \"<absolute>\"; }};",
		id(TEST_SOURCE),
		quote(&super::super::host_dir(root, Target::Ios).join("AppUITests.swift"))?
	)?;
	writeln!(
		objects,
		"{} = {{isa = PBXBuildFile; fileRef = {}; }};",
		id(TEST_SOURCE_BUILD),
		id(TEST_SOURCE)
	)?;
	writeln!(
		objects,
		"{} = {{isa = PBXFileReference; explicitFileType = wrapper.cfbundle; path = \
		 AppUITests.xctest; sourceTree = BUILT_PRODUCTS_DIR; }};",
		id(TEST_PRODUCT)
	)?;
	writeln!(
		objects,
		"{} = {{isa = PBXNativeTarget; buildConfigurationList = {}; buildPhases = ({}); \
		 buildRules = (); dependencies = ({}); name = AppUITests; productName = AppUITests; \
		 productReference = {}; productType = \"com.apple.product-type.bundle.ui-testing\"; }};",
		id(TEST_TARGET),
		id(TEST_CONFIGURATIONS),
		id(TEST_SOURCE_PHASE),
		id(TEST_DEPENDENCY),
		id(TEST_PRODUCT)
	)?;
	writeln!(
		objects,
		"{} = {{isa = PBXSourcesBuildPhase; buildActionMask = 2147483647; files = ({}); \
		 runOnlyForDeploymentPostprocessing = 0; }};",
		id(TEST_SOURCE_PHASE),
		id(TEST_SOURCE_BUILD)
	)?;
	writeln!(
		objects,
		"{} = {{isa = PBXTargetDependency; target = {}; targetProxy = {}; }};",
		id(TEST_DEPENDENCY),
		id(APP_TARGET),
		id(TEST_PROXY)
	)?;
	writeln!(
		objects,
		"{} = {{isa = PBXContainerItemProxy; containerPortal = {}; proxyType = 1; \
		 remoteGlobalIDString = {}; remoteInfo = App; }};",
		id(TEST_PROXY),
		id(PROJECT),
		id(APP_TARGET)
	)?;
	writeln!(
		objects,
		"{} = {{isa = XCConfigurationList; buildConfigurations = ({},{}); \
		 defaultConfigurationIsVisible = 0; defaultConfigurationName = Debug; }};",
		id(TEST_CONFIGURATIONS),
		id(TEST_DEBUG),
		id(TEST_RELEASE)
	)?;

	for (offset, name) in [(0, "Debug"), (1, "Release")] {
		writeln!(
			objects,
			"{} = {{isa = XCBuildConfiguration; name = {name}; buildSettings = {{ PRODUCT_NAME = \
			 AppUITests; PRODUCT_BUNDLE_IDENTIFIER = \"{package}.uitests\"; \
			 GENERATE_INFOPLIST_FILE = YES; SWIFT_VERSION = 6.0; TEST_TARGET_NAME = App; \
			 TARGETED_DEVICE_FAMILY = \"1,2\"; }}; }};",
			id(TEST_DEBUG + offset)
		)?;
	}

	Ok(())
}
/// Rust services own their capability registration; Swift imports only platform
/// plugins.
fn ios_registrant(registry: &Registry) -> Result<String> {
	let mut swift = String::from(
		"// Generated by leptoskit.\nimport CLeptosKitBridge\nimport LeptosKitBridge\n",
	);

	for plugin in &registry.plugins {
		if !plugin.has_rust_registration() {
			writeln!(
				swift,
				"import {}",
				plugin
					.apple_module
					.as_deref()
					.context("missing Apple module")?
			)?;
		}
	}

	swift.push_str(
		"\n@MainActor\nenum GeneratedPlugins {\n    static func register(_ bridge: OpaquePointer, \
		 _ dispatcher: NativeDispatcher, _ context: NativePluginContext) throws {\n",
	);

	for plugin in &registry.plugins {
		if !plugin.has_rust_registration() {
			writeln!(
				swift,
				"        try dispatcher.register({}({}), on: bridge)",
				plugin.apple_type.as_deref().context("missing Swift type")?,
				if plugin.apple_context {
					"context: context"
				} else {
					""
				}
			)?;
		}
	}

	swift.push_str("    }\n}\n");
	Ok(swift)
}
/// Stable package destinations preserve local bridge dependencies after
/// publication.
fn ios_packages(
	registry: &Registry,
	package_root: &Path,
	packages: &mut Vec<(std::path::PathBuf, String)>,
) -> Result<()> {
	for plugin in &registry.plugins {
		if !plugin.has_rust_registration() {
			let directory = package_root.join(&plugin.package_name);
			copy_package(&plugin.directory, &directory)?;
			packages.push((
				directory,
				plugin.apple_module.clone().context("missing module")?,
			));
		}
	}

	for helper in &registry.support {
		if let Some(module) = &helper.apple_module {
			let directory = package_root.join(&helper.package_name);
			copy_package(&helper.directory, &directory)?;
			packages.push((directory, module.clone()));
		}
	}

	Ok(())
}
fn ios_sources(
	root: &Path,
	registry: &Registry,
	dir: &Path,
	objects: &mut String,
) -> Result<(String, String)> {
	let sources = std::iter::once(super::super::host_dir(root, Target::Ios).join("App.swift"))
		.chain(
			platform_sources::IOS
				.iter()
				.map(|path| registry.bridge.join(path)),
		)
		.chain(std::iter::once(dir.join("GeneratedPlugins.swift")));
	let mut source_files = String::new();
	let mut source_refs = String::new();

	for (index, path) in sources.enumerate() {
		let file = id(SOURCE_START + index);
		let build = id(SOURCE_BUILD_START + index);
		writeln!(
			objects,
			"{file} = {{isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = {}; \
			 sourceTree = \"<absolute>\"; }};",
			quote(&path)?
		)?;
		writeln!(
			objects,
			"{build} = {{isa = PBXBuildFile; fileRef = {file}; }};"
		)?;
		write!(source_files, "{build},")?;
		write!(source_refs, "{file},")?;
	}

	Ok((source_files, source_refs))
}
fn ios_configurations(
	objects: &mut String,
	root: &Path,
	config: &leptoskit_config::LeptoskitConfig,
	package_root: &Path,
) -> Result<()> {
	for (offset, name) in [(0, "Debug"), (1, "Release")] {
		writeln!(
			objects,
			"{} = {{isa = XCBuildConfiguration; name = {name}; buildSettings = {{ \
			 CLANG_ENABLE_MODULES = YES; SDKROOT = iphoneos; IPHONEOS_DEPLOYMENT_TARGET = 17.0; \
			 }}; }};",
			id(PROJECT_DEBUG + offset)
		)?;
		let release_strip = if offset == 1 {
			" DEAD_CODE_STRIPPING = YES; DEPLOYMENT_POSTPROCESSING = YES; STRIP_INSTALLED_PRODUCT \
			 = YES; STRIP_STYLE = all;"
		} else {
			""
		};

		writeln!(
			objects,
			"{} = {{isa = XCBuildConfiguration; name = {name}; buildSettings = {{ PRODUCT_NAME = \
			 App; PRODUCT_BUNDLE_IDENTIFIER = {}; INFOPLIST_FILE = {}; SWIFT_VERSION = 6.0; \
			 TARGETED_DEVICE_FAMILY = \"1,2\"; SUPPORTED_PLATFORMS = \"iphoneos \
			 iphonesimulator\"; CODE_SIGN_STYLE = Automatic; LIBRARY_SEARCH_PATHS = \
			 (\"$(inherited)\",{}); SWIFT_INCLUDE_PATHS = {}; OTHER_SWIFT_FLAGS = \
			 (\"$(inherited)\",\"-I\",{}); LD_RUNPATH_SEARCH_PATHS = \
			 (\"$(inherited)\",\"@executable_path/Frameworks\");{release_strip} }}; }};",
			id(APP_DEBUG + offset),
			config.native.package_id,
			quote(&super::super::host_dir(root, Target::Ios).join("Info.plist"))?,
			quote(&super::super::generated_dir(root, Target::Ios).join("lib"))?,
			quote(&package_root.join("leptoskit_bridge_ffi/include"))?,
			quote(&package_root.join("leptoskit_bridge_ffi/include"))?
		)?;
	}

	Ok(())
}
fn id(number: usize) -> String {
	format!("{number:024X}")
}
const PROJECT: usize = 1;
const ROOT_GROUP: usize = 2;
const APP_TARGET: usize = 3;
const SOURCE_PHASE: usize = 4;
const RESOURCE_PHASE: usize = 5;
const FRAMEWORK_PHASE: usize = 6;
const PROJECT_CONFIGURATIONS: usize = 7;

const APP_CONFIGURATIONS: usize = 8;
const BUNDLE_FILE: usize = 30;
const BUNDLE_BUILD: usize = 31;
const APP_PRODUCT: usize = 32;
const UPDATE_FILE: usize = 34;
const UPDATE_BUILD: usize = 35;

const PROJECT_DEBUG: usize = 40;
const PROJECT_RELEASE: usize = 41;
const APP_DEBUG: usize = 42;
const APP_RELEASE: usize = 43;

const TEST_SOURCE: usize = 50;
const TEST_SOURCE_BUILD: usize = 51;
const TEST_PRODUCT: usize = 52;
const TEST_TARGET: usize = 53;

const TEST_CONFIGURATIONS: usize = 54;
const TEST_SOURCE_PHASE: usize = 55;
const TEST_DEPENDENCY: usize = 56;
const TEST_PROXY: usize = 57;
const TEST_DEBUG: usize = 58;
const TEST_RELEASE: usize = 59;

const PACKAGE_START: usize = 100;
const PRODUCT_START: usize = 200;
const PACKAGE_BUILD_START: usize = 300;
const SOURCE_START: usize = 1000;
const SOURCE_BUILD_START: usize = 2000;

fn package_references(
	registry: &Registry,
	package_root: &Path,
	bridge_package: std::path::PathBuf,
	objects: &mut String,
) -> Result<(String, String, String)> {
	let mut package_refs = String::new();
	let mut products = String::new();
	let mut frameworks = String::new();
	let mut packages = vec![(bridge_package, "LeptosKitBridge".to_owned())];

	ios_packages(registry, package_root, &mut packages)?;

	for (index, (path, product)) in packages.iter().enumerate() {
		let package_id = id(PACKAGE_START + index);
		let product_id = id(PRODUCT_START + index);
		let build_id = id(PACKAGE_BUILD_START + index);
		writeln!(
			objects,
			"{package_id} = {{isa = XCLocalSwiftPackageReference; relativePath = {}; \
			 }};\n{product_id} = {{isa = XCSwiftPackageProductDependency; package = {package_id}; \
			 productName = {product}; }};\n{build_id} = {{isa = PBXBuildFile; productRef = \
			 {product_id}; }};",
			quote(path)?
		)?;
		write!(package_refs, "{package_id},")?;
		write!(products, "{product_id},")?;
		write!(frameworks, "{build_id},")?;
	}

	Ok((package_refs, products, frameworks))
}

fn build_phases(
	objects: &mut String,
	root: &Path,
	config: &leptoskit_config::LeptoskitConfig,
	source_refs: &str,
	source_files: &str,
	frameworks: &str,
) -> Result<()> {
	writeln!(
		objects,
		"{} = {{isa = PBXFileReference; explicitFileType = wrapper.application; path = App.app; \
		 sourceTree = BUILT_PRODUCTS_DIR; }};",
		id(APP_PRODUCT)
	)?;
	writeln!(
		objects,
		"{} = {{isa = PBXGroup; children = ({source_refs}{},{},{},{},{}); sourceTree = \
		 \"<group>\"; }};",
		id(ROOT_GROUP),
		id(BUNDLE_FILE),
		id(UPDATE_FILE),
		id(APP_PRODUCT),
		id(TEST_SOURCE),
		id(TEST_PRODUCT)
	)?;
	ios_tests(objects, root, &config.native.package_id)?;
	writeln!(
		objects,
		"{} = {{isa = PBXSourcesBuildPhase; buildActionMask = 2147483647; files = \
		 ({source_files}); runOnlyForDeploymentPostprocessing = 0; }};",
		id(SOURCE_PHASE)
	)?;
	writeln!(
		objects,
		"{} = {{isa = PBXResourcesBuildPhase; buildActionMask = 2147483647; files = ({},{}); \
		 runOnlyForDeploymentPostprocessing = 0; }};",
		id(RESOURCE_PHASE),
		id(BUNDLE_BUILD),
		id(UPDATE_BUILD)
	)?;
	writeln!(
		objects,
		"{} = {{isa = PBXFrameworksBuildPhase; buildActionMask = 2147483647; files = \
		 ({frameworks}); runOnlyForDeploymentPostprocessing = 0; }};",
		id(FRAMEWORK_PHASE)
	)?;

	Ok(())
}

fn targets(objects: &mut String, products: &str, package_refs: &str) -> Result<()> {
	writeln!(
		objects,
		"{} = {{isa = XCConfigurationList; buildConfigurations = ({},{}); \
		 defaultConfigurationIsVisible = 0; defaultConfigurationName = Debug; }};",
		id(PROJECT_CONFIGURATIONS),
		id(PROJECT_DEBUG),
		id(PROJECT_RELEASE)
	)?;
	writeln!(
		objects,
		"{} = {{isa = XCConfigurationList; buildConfigurations = ({},{}); \
		 defaultConfigurationIsVisible = 0; defaultConfigurationName = Debug; }};",
		id(APP_CONFIGURATIONS),
		id(APP_DEBUG),
		id(APP_RELEASE)
	)?;
	writeln!(
		objects,
		"{} = {{isa = PBXNativeTarget; buildConfigurationList = {}; buildPhases = ({},{},{}); \
		 buildRules = (); dependencies = (); name = App; productName = App; productReference = \
		 {}; productType = \"com.apple.product-type.application\"; packageProductDependencies = \
		 ({products}); }};",
		id(APP_TARGET),
		id(APP_CONFIGURATIONS),
		id(SOURCE_PHASE),
		id(RESOURCE_PHASE),
		id(FRAMEWORK_PHASE),
		id(APP_PRODUCT)
	)?;
	writeln!(
		objects,
		"{} = {{isa = PBXProject; attributes = {{ LastUpgradeCheck = 1600; }}; \
		 buildConfigurationList = {}; compatibilityVersion = \"Xcode 14.0\"; developmentRegion = \
		 en; knownRegions = (en,Base); mainGroup = {}; projectDirPath = \"\"; projectRoot = \"\"; \
		 targets = ({},{}); packageReferences = ({package_refs}); }};",
		id(PROJECT),
		id(PROJECT_CONFIGURATIONS),
		id(ROOT_GROUP),
		id(APP_TARGET),
		id(TEST_TARGET)
	)?;
	Ok(())
}
