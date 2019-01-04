**Update January 4, 2019**

The major refactor and rewrite in Qt is done. There is no longer a need for a separate CLI branch. The Qt version requires external files, but I wanted to keep it a standalone binary, so I've written a wrapper that outputs everything to `$temp` and runs from there. The code is much improved and the program should be stabler and easier to use. I'm calling this version 2.0 and will be working through the planned features mentioned below soon. 

# Flying Carpet

To download, visit the ![releases](https://github.com/spieglt/FlyingCarpet/releases) page!

Wireless, encrypted file transfer over automatically configured ad hoc networking. No network infrastructure required (access point, router, switch). Just two laptops (Mac, Linux, and Windows supported) with wireless chips in close range.

Don't have a flash drive? Don't have access to a wireless network or don't trust one? Need to move a file larger than 2GB between different filesystems but don't want to set up a file share? Try it out!

# Screenshots:

![](pictures/linuxDemo.png)  ![](pictures/winDemo.png)  ![](pictures/macDemo.png)

# Features:

+ Cross-platform: Linux, Mac, and Windows.

+ Transfer multiple files at once, without losing progress if the transfer is interrupted or canceled.

+ Speeds over 120mbps (with laptops close together).

+ Does not use Bluetooth or your local network, just wireless chip to wireless chip.

+ Files encrypted in transit.

+ Large files supported (<10MB RAM usage while transferring a 4.5GB file).

+ Standalone executable, no installation required and no dependencies needed.

+ Interoperable GUI and CLI versions.

# GUI Compilation instructions:

+ `go get -x github.com/spieglt/flyingcarpet`

+ Windows only: Compile WFD_DLL project with Visual Studio (Release, x64 mode) and then run `makeIconSyso.bat` while in `icons/Windows` folder.

+ If compiling on Windows, get `mt.exe` (available in Windows SDKs) and make sure it's in your path.

+ Set up ![therecipe/qt](https://github.com/therecipe/qt/wiki/Installation) and make sure `qtdeploy` is in your path.

+ Install ![go.rice](https://github.com/GeertJohan/go.rice) and make sure `rice` is in your path.

+ Optional: get `windres.exe` if you want to change the icon on Windows (available in MinGW).

+ Run `.\wg_rebuild.ps1` from Powershell (for Windows), `./mg_rebuild` from Terminal (for Mac), or `./lg_rebuild` (for Linux).

# CLI Compilation instructions

+ `go get -x github.com/spieglt/flyingcarpet`

+ Windows only: Install ![go.rice](https://github.com/GeertJohan/go.rice) and make sure `rice` is in your path. 

+ Windows only: Compile WFD_DLL project with Visual Studio (Release, x64 mode). Then copy `flyingcarpet\WFD_DLL\x64\Release\WFD_DLL.dll` to `flyingcarpet\cli\static`, change directory to `flyingcarpet\cli`, and run `rice.exe embed-go`

+ `cd $GOPATH/src/github.com/spieglt/flyingcarpet/cli`

+ `go build -o flyingcarpet.exe`

(Note: CLI version is not working on Windows due to needing to rewrite DLL embedding. Will be fixed soon.)

# Restrictions:

+ 64-bit only. Supported Operating Systems: macOS 10.12+, Windows 7+, and Linux Mint 18. I only have access to so many laptops, so if you've tried on other platforms please let me know whether it worked. 

+ Disables your wireless internet connection while in use (does not apply to Windows when receiving).

+ On Mac: May have to click Allow or enter username and password at prompt to clear Flying Carpet SSID from your preferred networks list. You may also have to right-click and select "Open" if your settings don't allow running unsigned applications. 

+ On Windows: Must run as administrator (to allow connection through firewall and clear ARP cache). Right-click "Flying Carpet.exe" and select "Run as administrator." Click "More info" and "Run anyway" if you receive a Windows SmartScreen prompt. You may also need to disable WiFi Sense.

+ I need help testing on Linux and supporting non-Debian-based distributions! Currently only confirmed to work on Mint 18.

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
