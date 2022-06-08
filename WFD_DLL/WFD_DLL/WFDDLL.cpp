// WFDDLL.cpp : Defines the exported functions for the DLL application.
//

#include "stdafx.h"
#include "WlanHostedNetworkWinRT.h"
#include "SimpleConsole.h"
#include "WFDDLL.h"

using namespace ABI::Windows::Foundation;
using namespace Microsoft::WRL;
using namespace Microsoft::WRL::Wrappers;

SimpleConsole * console = NULL;
HANDLE uninitializeRuntime;
HANDLE runtimeInitFailure;
HANDLE runtimeInitSuccess;
HANDLE runtimeInitEvents[2];
HANDLE runtimeThread;

DWORD InitRuntime(LPVOID lpParam) {

	UNREFERENCED_PARAMETER(lpParam);

	// Start Windows Runtime
	RoInitializeWrapper initialize(RO_INIT_MULTITHREADED);
	if (FAILED(initialize))
	{
		// Signal failure.
		if (!SetEvent(runtimeInitFailure)) {
			return GetLastError();
		}
		return static_cast<HRESULT>(initialize);
	}
	// Signal success.
	if (!SetEvent(runtimeInitSuccess)) {
		return GetLastError();
	}

	// Wait here until WiFi Direct is done.
	DWORD dwResult = WaitForSingleObject(uninitializeRuntime, INFINITE);
	switch (dwResult)
	{
	// Event object was signaled
	case WAIT_OBJECT_0:
		break;
	// An error occurred
	default:
		return 0;
	}
	return 1;
}

// Put C functions callable from Go here.

int __cdecl GoConsoleInit() {
	// Create event used to stop runtime later.
	uninitializeRuntime = CreateEvent(
		NULL,	// Default security
		FALSE,  // Manual reset false, resets after one thread is released
		FALSE,  // Initial state false
		L"Uninitialize Windows Runtime."
	);
	// Create event for when runtime initialization fails.
	runtimeInitFailure = CreateEvent(
		NULL,	// Default security
		FALSE,  // Manual reset false, resets after one thread is released
		FALSE,  // Initial state false
		L"Windows runtime initialization failed."
	);
	// Create event for when runtime initialization succeeds.
	runtimeInitSuccess = CreateEvent(
		NULL,	// Default security
		FALSE,  // Manual reset false, resets after one thread is released
		FALSE,  // Initial state false
		L"Windows runtime initialization succeded."
	);
	runtimeInitEvents[0] = runtimeInitFailure;
	runtimeInitEvents[1] = runtimeInitSuccess;

	// Start runtime in background thread.
	DWORD runtimeThreadID;
	runtimeThread = CreateThread(
		NULL,              // default security
		0,                 // default stack size
		InitRuntime,       // name of the thread function
		NULL,              // no thread parameters
		0,                 // default startup flags
		&runtimeThreadID
	);

	// Wait for success or failure of Windows Runtime initialization.
	DWORD dwResult = WaitForMultipleObjects(
		2,					// number of objects, success or failure
		runtimeInitEvents,	// array with the two objects
		FALSE,				// don't wait for both, cease blocking on either
		INFINITE			// wait indefinitely
	);
	if (dwResult - WAIT_OBJECT_0 == 0) {	// 0 == index of failure event
		return 0;
	}
	if (dwResult - WAIT_OBJECT_0 == 1) {
		console = new SimpleConsole();	// 1 == index of success event
		return 1;
	}
	// something went wrong if we get here
	return 5;
}

int GoConsoleFree() {
	// Free all runtime components.
	delete console;
	// Send signal to InitRuntime to exit.
	if (!SetEvent(uninitializeRuntime))
	{
		printf("SetEvent failed (%d)\n", GetLastError());
		return 0;
	}
	return 1;
}

void GoConsoleExecuteCommand(void * pCmd) {
	char * cmd = (char*)pCmd;
	size_t wSize = strlen(cmd) + 1;
	wchar_t * wCmd = new wchar_t[wSize];
	size_t num;
	mbstowcs_s(&num, wCmd, wSize, cmd, _TRUNCATE);
	std::wstring command(wCmd);
	console->ExecuteCommand(command);
	delete wCmd;
}
