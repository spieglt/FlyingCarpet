# Flying Carpet
Wireless, encrypted file transfer over automatically configured ad hoc networking. No network infrastructure required (access point, router, switch). Just two laptops (Mac and/or Windows) with wireless chips in close range.

Don't have a flash drive? Don't have access to a wireless network or don't trust one? Need to move a file larger than 2GB between Mac and Windows but don't want to set up a file share? Try it out!

# Sample Usage
**On receiving end (Mac):**

`./flyingcarpet -receive transferred_movie.avi -peer windows`

*\[Write down password\]*

**On sending end (Windows):**

`flyingcarpet.exe -send movie.avi -peer mac`

*\[Enter password from Mac\]*

# Features:
+ Cross-platform, Mac and Windows.

+ Speeds over 120mbps (with laptops close together).

+ Does not use Bluetooth or your local network, just wireless chip to wireless chip.

+ Files encrypted in transit.

+ Large files supported.

+ Standalone binary, no installation required.

# Compilation instructions:
`cd flyingcarpet`

`go get ./...`

`go build`

# Restrictions:
+ Disables your wireless internet connection while in use (does not apply to Windows when receiving)

+ On Mac: May have to click Allow or enter username and password at prompt to join ad-hoc network.

+ On Windows: May have to allow TCP listener through firewall

+ Windows laptop must support hosted networking. To find out if yours does, run `netsh wlan show drivers`. If the `Hosted network supported : ` line says `No`, you can't use this product. Known issue on Surface Pro 3 and later.

+ If you choose to receive a filename that is already present in your current directory, it will be overwritten.

+ After a successful transfer, Flying Carpet will attempt to rejoin you to your previous wireless networks. If there is an error midway through the process, this may fail.

+ GUI forthcoming.

Disclaimer: I am not a cryptography expert. This is a usable product in its current state, but is also a learning experience for me and a work in progress. Do not use for private files if you think a skilled hacker is less than 100 feet from you and trying to intercept them.

Licenses for third-party tools and libraries used can be found in the "3rd_party_licenses" folder.
