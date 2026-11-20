# Yoink

An ad-hoc package manager for Linux, in Rust with a [Slint](https://slint.dev)
interface in [Chippy](https://github.com/singularity098/Chippy)'s dark style.
Add an app by its GitHub repository and Yoink fetches its newest build, from
**Releases** or from **Actions** (dev builds), and puts it where it belongs:

| Kind | Where it goes |
| --- | --- |
| AppImage | Your AppImages folder. An AppImage inside a zip is unpacked and the zip discarded; the previous version's AppImage is deleted (or overwritten, same name). |
| Binary | A lone executable goes to your bin folder. A package laid out as `bin/`, `lib/`, `share/` goes to your bin, lib and share folders. The archive is kept in the packages folder; the previous one is removed. |
| Plugin | `.vst3` and `.lv2` bundles and `.clap` plugins go to their folders, replacing older versions. VST2 plugins are plain `.so` files, so you pick them once. The archive is kept in the packages folder. |
| APK | Saved to your APK folder (for syncing to your phone), replacing the previous APK. Installing it on the phone is up to you. |

AppImages and binaries can get a `.desktop` entry: the one inside the
AppImage (pointed at where it was delivered), or one Yoink writes.

Nothing needs root, and nothing is installed without showing you first:
updating downloads the builds into a cache, checks them (SHA-256, when the
release publishes one), unpacks them and looks inside, then lists every
file that will be downloaded, copied, replaced, kept and deleted. Only
**Apply** changes anything.

## Using it

* **Add** an app: type `owner/name` (or paste the repository's URL). Yoink
  lists the files the newest release (or Actions run) offers and picks the
  likeliest one for your computer; pick another if you like. The file
  becomes a pattern with its version numbers as `*`, so later versions match.
* **Several deployments** per app: a plugin and its standalone build, the
  desktop app and its Android APK, each from Releases or Actions.
* **Actions builds**: pick the workflow (or any) and the branch (or the
  default one). Yoink takes the newest run that has your file: if a run is
  still in progress but your file is already uploaded (the Linux build done
  while Windows is still compiling), it uses it; if not, it uses the run
  before and tells you a newer build is coming. Runs that failed are skipped
  unless you allow them in Options. GitHub only lets Actions builds be
  downloaded with a token.
* **Check for updates** (F5), **Update all**, **Update** one app, or
  **Force update** (reinstall) from its ⋮ menu.
* **Remove** an app: delete everything it put in place (shown first), or
  just stop managing it.
* **Options** (the gear, or `,`) slide out from the right: the folders, the
  GitHub token (kept in your keyring), the theme, and Yoink's own updates.
* **Yoink in the applications menu**: Options → Yoink → *Show Yoink in the
  applications menu* writes `~/.local/share/applications/yoink.desktop` and
  its icon, pointed at the AppImage (and kept pointed at it if it moves);
  turning it off removes them. From a terminal: `yoink --install-desktop`
  and `yoink --remove-desktop`. An entry a package installed is left alone.

Yoink records every file each deployment puts in place, so updates and
removals delete exactly those, and a file that Yoink did not put there (or
that belongs to another app) is flagged in the plan before it would be
overwritten.

### The GitHub token

Without one, GitHub allows 60 requests an hour and no Actions downloads. A
fine-grained token with read access to *Contents* and *Actions* (or a
classic token with no scopes, for public repositories) is enough. It is
stored in the desktop's keyring (Secret Service), or in
`~/.config/yoink/token` (readable only by you) where there is none.

### Yoink updating itself

Yoink is distributed as an AppImage and can replace its own AppImage, from
Releases or from Actions builds (Options → Yoink), then restart.

## Files

* `~/.config/yoink/settings.toml`: the settings.
* `~/.local/share/yoink/library.json`: the apps, their deployments and what
  each installed.
* `~/.cache/yoink/staging`: downloads waiting for a plan to be applied.

`YOINK_HOME=/some/folder` keeps all three below one folder instead.

## Building

```bash
cargo build --release
```

Needs Rust 1.88 or newer, and the fontconfig and xkbcommon development
packages. Skia renders by default; `--no-default-features` uses femtovg.

`yoink --probe owner/name [releases|pre|actions[:BRANCH]] [PATTERN]` shows what a
repository offers and which build a pattern resolves to, without
downloading anything. `yoink --inspect FILE` shows what Yoink finds inside
a package.

## Android

`android/` holds Yoink for Android phones: a smaller Yoink (Kotlin and
Jetpack Compose, in the same look and themes) that only deals in APKs, and
installs them itself instead of saving them for syncing.

* **Add** an app by its repository, from Releases (or pre-releases) or
  Actions builds, as on the desktop. Yoink picks the APK for the phone's
  processor (`arm64-v8a`, `armeabi-v7a`, `x86_64`…, or a universal one)
  and makes a pattern of its name. Actions builds come zipped: the APK is
  taken out of the zip, and an APK inside a zipped release is too.
* **Check for updates** (the refresh button, or pull down), **Update**,
  **Update all**, **Force update** (reinstall). Each shows what it will
  download first, checks the SHA-256 when the release publishes one, then
  hands the APK to Android's package installer. Android asks to confirm
  the first install; updates of apps Yoink installed go through without
  asking on Android 12 and newer. Android refuses an APK signed with
  another key than the installed app, or an older version.
* **Remove** an app: uninstall it (Android asks), or only stop managing it.
* **Options**: the GitHub token (encrypted with a key in the Android
  Keystore), allowing failed Actions runs, the *Install unknown apps*
  permission Yoink needs, and the theme.
* **Yoink updating itself**: add `singularity098/Yoink` and pick
  `Yoink-android.apk` (Releases, or the Actions artifact for dev builds).

Needs Android 8 or newer. Split APK bundles (`.apks`, `.xapk`) are not
supported.

### Building

```bash
cd android
./gradlew assembleRelease     # app/build/outputs/apk/release/app-release.apk
./gradlew testDebugUnitTest
```

Needs JDK 17 or newer and the Android SDK (platform 36), found through
`ANDROID_HOME` or `local.properties`. A release build is signed with the
keystore named by `YOINK_KEYSTORE` (with `YOINK_KEYSTORE_PASSWORD`,
`YOINK_KEY_ALIAS` and `YOINK_KEY_PASSWORD`), or with the debug key.

CI builds `Yoink-android.apk` on every push. To sign every build with one
key, so each installs as an update of the last, add the repository secrets
`ANDROID_KEYSTORE_BASE64` (the keystore, `base64 -w0 yoink.jks`),
`ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS` and `ANDROID_KEY_PASSWORD`.
Make the keystore once, and keep it safe:

```bash
keytool -genkeypair -keystore yoink.jks -alias yoink -keyalg RSA -keysize 4096 -validity 10000
```

## Licence

GPL-3.0-or-later. The look, the icons and the Jost font (SIL Open Font
Licence) come from Chippy.
