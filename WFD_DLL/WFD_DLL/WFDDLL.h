#pragma once

DWORD WINAPI InitRuntime(LPVOID lpParam);
extern "C" __declspec(dllexport) int GoConsoleInit();
extern "C" __declspec(dllexport) int GoConsoleFree();
extern "C" __declspec(dllexport) void GoConsoleExecuteCommand(void*);
