$ErrorActionPreference = "Stop"

$projectRoot = Split-Path -Parent $PSScriptRoot
$ndkRoot = "C:\Users\29488\AndroidNDK\android-ndk-r27d"
$adbSerial = "127.0.0.1:7555"
$target = "x86_64-linux-android"
$binaryName = "projectying"
$localBinary = Join-Path $projectRoot "target\$target\release\$binaryName"
$remoteTmpBinary = "/data/local/tmp/$binaryName"
$remoteTermuxDir = "/data/data/com.termux/files/home/projectying"
$remoteRunScript = "$remoteTermuxDir/run-android-x86_64.sh"
$localRunScript = Join-Path $env:TEMP "run-android-x86_64.sh"

if (-not (Test-Path $ndkRoot)) {
    throw "Android NDK not found: $ndkRoot"
}

Write-Host "==> Connecting ADB"
adb connect $adbSerial | Out-Host

Write-Host "==> Building $target"
$env:ANDROID_NDK_HOME = $ndkRoot
$env:CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER = "$ndkRoot\toolchains\llvm\prebuilt\windows-x86_64\bin\x86_64-linux-android24-clang.cmd"
$env:CARGO_TARGET_X86_64_LINUX_ANDROID_AR = "$ndkRoot\toolchains\llvm\prebuilt\windows-x86_64\bin\llvm-ar.exe"
$env:CC_x86_64_linux_android = $env:CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER
$env:AR_x86_64_linux_android = $env:CARGO_TARGET_X86_64_LINUX_ANDROID_AR
$env:CXX_x86_64_linux_android = "$ndkRoot\toolchains\llvm\prebuilt\windows-x86_64\bin\x86_64-linux-android24-clang++.cmd"

Push-Location $projectRoot
try {
    cargo build --release --target $target
} finally {
    Pop-Location
}

if (-not (Test-Path $localBinary)) {
    throw "Build output not found: $localBinary"
}

Write-Host "==> Preparing Termux launcher"
@'
#!/system/bin/sh
unset LD_PRELOAD
unset LD_LIBRARY_PATH
HOME=/data/data/com.termux/files/home
PATH=/data/data/com.termux/files/usr/bin:/system/bin
export HOME PATH
cd /data/data/com.termux/files/home/projectying
exec /data/local/tmp/projectying "$@"
'@ | Set-Content -LiteralPath $localRunScript -NoNewline

Write-Host "==> Pushing binary and launcher"
adb -s $adbSerial push $localBinary $remoteTmpBinary | Out-Host
adb -s $adbSerial push $localRunScript /data/local/tmp/run-android-x86_64.sh | Out-Host

Write-Host "==> Installing launcher into Termux home"
adb -s $adbSerial shell "run-as com.termux /system/bin/cp /data/local/tmp/run-android-x86_64.sh $remoteRunScript && run-as com.termux /system/bin/chmod 755 $remoteRunScript" | Out-Host

Write-Host "==> Verifying deployed binary"
adb -s $adbSerial shell "run-as com.termux /system/bin/sh -c '$remoteRunScript --version'" | Out-Host

Write-Host ""
Write-Host "Done. In Termux run:"
Write-Host "  cd /data/data/com.termux/files/home/projectying"
Write-Host "  ./run-android-x86_64.sh"
