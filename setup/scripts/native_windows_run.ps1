$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force target/native-ui/windows | Out-Null
$csc = "$env:WINDIR/Microsoft.NET/Framework64/v4.0.30319/csc.exe"
$fixtureSource = (Resolve-Path 'crates/leptoskit_testing/fixtures/windows/CaptureLab.cs').Path
$fixtureExe = [System.IO.Path]::GetFullPath('target/native-ui/windows/CaptureLab.exe')
& $csc /nologo /target:winexe /reference:System.Windows.Forms.dll /reference:System.Drawing.dll "/out:$fixtureExe" $fixtureSource
if ($LASTEXITCODE -ne 0) { throw 'Native fixture compilation failed' }
Set-Content target/native-ui/windows/evidence.txt 'Leptoskit native file chooser evidence'
npm ci --prefix setup/native-ui
if ($LASTEXITCODE -ne 0) { throw 'Appium installation failed' }
$previousAppiumHome = $env:APPIUM_HOME
Remove-Item Env:APPIUM_HOME -ErrorAction SilentlyContinue
try {
    $server = Start-Process node -WorkingDirectory (Resolve-Path setup/native-ui) -ArgumentList @('node_modules/appium/build/lib/main.js', '--address', '127.0.0.1', '--port', '14731', '--use-drivers', 'novawindows', '--log-no-colors') -PassThru -RedirectStandardOutput target/native-ui/appium-windows.log -RedirectStandardError target/native-ui/appium-windows-error.log
} finally { $env:APPIUM_HOME = $previousAppiumHome }
try {
    foreach ($decision in @('deny', 'allow')) {
        $fixture = Start-Process (Resolve-Path target/native-ui/windows/CaptureLab.exe) -PassThru
        try {
            & target/debug/examples/native_conformance.exe windows http://127.0.0.1:14731 Root desktop $decision
            if ($LASTEXITCODE -ne 0) { throw "Native Windows $decision journey failed" }
        } finally { Stop-Process -Id $fixture.Id -ErrorAction SilentlyContinue }
    }
    # The dialog plugin's own fixture presents its real file chooser and
    # message box; the chooser opens beside the fixture executable.
    cargo build --locked -p leptoskit_plugin_dialog --example native_fixture
    if ($LASTEXITCODE -ne 0) { throw 'Dialog plugin fixture build failed' }
    cargo build --locked -p leptoskit_testing --features native-ui --example dialog_plugin_journey
    if ($LASTEXITCODE -ne 0) { throw 'Dialog plugin journey build failed' }
    Set-Content target/debug/examples/evidence.txt 'Leptoskit dialog plugin evidence'
    foreach ($decision in @('decline', 'accept')) {
        $fixture = Start-Process (Resolve-Path target/debug/examples/native_fixture.exe) -PassThru
        try {
            & target/debug/examples/dialog_plugin_journey.exe http://127.0.0.1:14731 $decision
            if ($LASTEXITCODE -ne 0) { throw "Dialog plugin $decision journey failed" }
        } finally { Stop-Process -Id $fixture.Id -ErrorAction SilentlyContinue }
    }
} finally { Stop-Process -Id $server.Id -ErrorAction SilentlyContinue }
