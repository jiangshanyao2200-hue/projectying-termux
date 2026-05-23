#!/data/data/com.termux/files/usr/bin/bash
set -euo pipefail

APP_DIR="$(cd "$(dirname "$0")" && pwd)"
BUILD_DIR="$APP_DIR/build"
CLASS_DIR="$BUILD_DIR/classes"
DEX_DIR="$BUILD_DIR/dex"
DIST_DIR="$APP_DIR/dist"
KEYSTORE_DIR="$BUILD_DIR/keystore"
GEN_DIR="$BUILD_DIR/generated"
RES_FLAT_DIR="$BUILD_DIR/res-flat"
SDK_DIR="$APP_DIR/.sdk"
PLATFORM_DIR="$SDK_DIR/platforms/android-34-ext12"
PLATFORM_ZIP="$SDK_DIR/platform-34-ext12_r01.zip"
PLATFORM_URL="https://dl.google.com/android/repository/platform-34-ext12_r01.zip"
JAVAC_BIN="${JAVAC_BIN:-/data/data/com.termux/files/usr/lib/jvm/java-17-openjdk/bin/javac}"
AAPT2_BIN="${AAPT2_BIN:-aapt2}"
UNSIGNED_APK="$BUILD_DIR/aichat-unsigned.apk"
ALIGNED_APK="$BUILD_DIR/aichat-aligned.apk"
FINAL_APK="$DIST_DIR/ai-chat-debug.apk"
ANDROID_JAR="${ANDROID_JAR:-$PLATFORM_DIR/android.jar}"
KEYSTORE_PATH="$KEYSTORE_DIR/debug.keystore"
KEY_ALIAS="projectyingdebug"
KEY_PASS="android"

rm -rf "$CLASS_DIR" "$DEX_DIR" "$GEN_DIR" "$RES_FLAT_DIR"
mkdir -p "$CLASS_DIR" "$DEX_DIR" "$DIST_DIR" "$KEYSTORE_DIR" "$GEN_DIR" "$SDK_DIR" "$RES_FLAT_DIR"

if [ ! -f "$ANDROID_JAR" ]; then
  if [ -f "$APP_DIR/../agentbrowser/.sdk/platforms/android-34-ext12/android.jar" ]; then
    mkdir -p "$SDK_DIR/platforms"
    ln -sfn "$APP_DIR/../agentbrowser/.sdk/platforms/android-34-ext12" "$PLATFORM_DIR"
  else
    if [ ! -f "$PLATFORM_ZIP" ]; then
      curl -L --fail --retry 3 -o "$PLATFORM_ZIP" "$PLATFORM_URL"
    fi
    unzip -q -o "$PLATFORM_ZIP" -d "$SDK_DIR/platforms"
  fi
fi

find "$APP_DIR/res" -name '*.xml' | sort > "$BUILD_DIR/res_sources.txt"
while IFS= read -r res_file; do
  [ -n "$res_file" ] || continue
  out_dir="$RES_FLAT_DIR/$(dirname "${res_file#$APP_DIR/res/}")"
  mkdir -p "$out_dir"
  "$AAPT2_BIN" compile -o "$out_dir" "$res_file"
done < "$BUILD_DIR/res_sources.txt"

RES_LINK_ARGS=()
while IFS= read -r compiled; do
  RES_LINK_ARGS+=( -R "$compiled" )
done < <(find "$RES_FLAT_DIR" -name '*.flat' | sort)

find "$APP_DIR/src" -name '*.java' | sort > "$BUILD_DIR/java_sources.txt"
"$JAVAC_BIN" -source 7 -target 7 -encoding UTF-8 -cp "$ANDROID_JAR" -d "$CLASS_DIR" @"$BUILD_DIR/java_sources.txt"
jar --create --file "$BUILD_DIR/classes.jar" -C "$CLASS_DIR" .
d8 --release --min-api 26 --lib "$ANDROID_JAR" --output "$DEX_DIR" "$BUILD_DIR/classes.jar"

"$AAPT2_BIN" link \
  --manifest "$APP_DIR/AndroidManifest.xml" \
  -I "$ANDROID_JAR" \
  --min-sdk-version 26 \
  --auto-add-overlay \
  -A "$APP_DIR/assets" \
  "${RES_LINK_ARGS[@]}" \
  -o "$UNSIGNED_APK"

(cd "$DEX_DIR" && aapt add "$UNSIGNED_APK" classes.dex >/dev/null)
zipalign -f 4 "$UNSIGNED_APK" "$ALIGNED_APK"

if [ ! -f "$KEYSTORE_PATH" ]; then
  keytool -genkeypair -keystore "$KEYSTORE_PATH" -storepass "$KEY_PASS" -keypass "$KEY_PASS" -alias "$KEY_ALIAS" \
    -dname "CN=ProjectYing, OU=AItermux, O=ProjectYing, L=Unknown, S=Unknown, C=CN" -keyalg RSA -keysize 2048 -validity 10000 >/dev/null
fi

apksigner sign --ks "$KEYSTORE_PATH" --ks-pass "pass:$KEY_PASS" --key-pass "pass:$KEY_PASS" --ks-key-alias "$KEY_ALIAS" --out "$FINAL_APK" "$ALIGNED_APK"
apksigner verify --verbose "$FINAL_APK"
ls -lh "$FINAL_APK"
