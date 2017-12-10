copy C:\Users\Theron\source\repos\WFD_DLL\x64\Release\WFD_DLL.dll .\static\wfd.dll
.\go-bindata -o static.go static\
go build
.\flyingcarpet.exe