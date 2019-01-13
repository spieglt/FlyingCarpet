**Update January 10, 2019**

Flying Carpet 2.0 is ready to use on the [releases](https://github.com/spieglt/FlyingCarpet/releases) page! (Binaries for CLI version coming soon.)

It includes a GUI rewritten with Qt, a refactor uniting the CLI and GUI branches, retry logic, better admin handling on Windows and Mac, clearer instructions, and better filesystem and transfer code. Please send feedback if you use it and thank you for your interest.

Note for Windows users: I had tried to bundle the GUI version as a single `.exe` file, but it's not currently working. For now, please extract `Flying Carpet (Windows).zip` and run the `flyingcarpet.exe` inside.

# Flying Carpet

To download, visit the [releases](https://github.com/spieglt/FlyingCarpet/releases) page!

Wireless, encrypted file transfer over automatically configured ad hoc networking. No network infrastructure required (access point, router, switch). Just two laptops (Mac, Linux, and Windows supported) with wireless chips in close range.

Don't have a flash drive? Don't have access to a wireless network or don't trust one? Need to move a file larger than 2GB between different filesystems but don't want to set up a file share? Try it out!

# Screenshots:

<img src="pictures/winDemo.png" width=400> <img src="pictures/macDemo.png" width=400> <img src="pictures/linuxDemo.png" width=400> 

# Features:

+ Cross-platform: Linux, Mac, and Windows.

+ Transfer multiple files at once, without losing progress if the transfer is interrupted or canceled.

+ Speeds over 120mbps (with laptops close together).

+ Does not use Bluetooth or your local network, just wireless chip to wireless chip.

+ Files encrypted in transit.

+ Large files supported (<10MB RAM usage while transferring a 4.5GB file).

+ No installation required and no dependencies needed.

+ Interoperable GUI and CLI versions.

# GUI Compilation instructions:

+ `go get -x github.com/spieglt/flyingcarpet`

+ Windows only: Open `flyingcarpet\WFD_DLL\WFD_DLL.sln` with Visual Studio, and compile in Release mode for x64.

+ If compiling on Windows, get `mt.exe` (available in Windows SDKs) and make sure it's in your path.

+ Go through the entire setup guide for ![therecipe/qt](https://github.com/therecipe/qt/wiki/Installation) and make sure `qtdeploy` is in your path.

+ Install ![go.rice](https://github.com/GeertJohan/go.rice) and make sure `rice` is in your path.

+ Run `.\wg_rebuild.ps1` from Powershell (for Windows), `./mg_rebuild` from Terminal (for Mac), or `./lg_rebuild` (for Linux).

# CLI Compilation instructions

+ `go get -x github.com/spieglt/flyingcarpet`

+ Windows only: Open `flyingcarpet\WFD_DLL\WFD_DLL.sln` with Visual Studio, and compile in Release mode for x64. Then install ![go.rice](https://github.com/GeertJohan/go.rice) and make sure `rice` is in your path.

+ `cd $GOPATH/src/github.com/spieglt/flyingcarpet/cli`

+ Windows only: `.\embed_dll.ps1` from Powershell

+ `go build -o flyingcarpet.exe`

# Restrictions:

+ The Mac version is a standard `.app` bundle, the Linux version is an executable that writes dependencies to a temp location and runs from there, and the Windows version is a `.zip` with an `.exe` and other dependencies inside. I'm working on a better solution for Windows. It was a standalone `.exe` when I was using wxWidgets but this has not been possible since moving to Qt. PRs welcome.

+ 64-bit only. Supported Operating Systems: macOS 10.12+, Windows 7+, and Linux Mint 18. I only have access to so many laptops, so if you've tried on other platforms please let me know whether it worked. 

+ Disables your wireless internet connection while in use (does not apply to Windows when receiving).

+ On Mac: You may have to right-click and select "Open" if your settings don't allow running unsigned applications. 

+ On Windows: Click "More info" and "Run anyway" if you receive a Windows SmartScreen prompt. You may also need to disable WiFi Sense.

+ I need help testing on Linux and supporting non-Debian-based distributions! Currently only confirmed to work on Mint 18, and only on wireless cards/drivers that support ad hoc networking with `nmcli`.

+ Flying Carpet should rejoin you to your previous wireless network after a completed or canceled transfer. This will not happen if the program freezes, crashes, or if the windows is closed during operation.

# Planned features:

+ Drag and drop for sending files.

+ Folder upload.

+ Make CLI version easier to use.

+ Replace `netsh wlan` with Native WiFi API on Windows.

+ Mobile versions, integrating functionality from https://github.com/claudiodangelis/qr-filetransfer.

Disclaimer: I am not a cryptography expert. Do not use for private files if you think a skilled attacker is less than 100 feet from you and trying to intercept them.

Licenses for third-party tools and libraries used can be found in the "3rd_party_licenses" folder.

If you've used Flying Carpet, please send me feedback! Thank you for your interest!
