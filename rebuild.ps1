del '.\bin\Flying Carpet (Windows).zip'

Copy-Item .\WFD_DLL\x64\Release\WFD_DLL.dll .\static\wfd.dll
.\go-bindata -o static.go static\
go build

Copy-Item .\flyingcarpet.exe '.\Flying Carpet.exe'
Compress-Archive -Path '.\Flying Carpet.exe' -DestinationPath '.\bin\Flying Carpet (Windows).zip'

del .\flyingcarpet.exe

Start-Process '.\Flying Carpet.exe'