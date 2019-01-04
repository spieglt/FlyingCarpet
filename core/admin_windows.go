package core

/*
#include <stdio.h>
#include <windows.h>
#include <windowsx.h>
#include <strsafe.h>
#include <shlobj.h>

// These functions have been adapted from https://code.msdn.microsoft.com/windowsapps/CppUACSelfElevation-5bfc52dd.
// Return value of 2 means error.
// TODO: better error handling.
// DWORD == unsigned long == 32 bits on Windows == uint32.

DWORD IsUserInAdminGroup()
{
    BOOL fInAdminGroup = FALSE;
    DWORD dwError = ERROR_SUCCESS;
    HANDLE hToken = NULL;
    HANDLE hTokenToCheck = NULL;
    DWORD cbSize = 0;
    OSVERSIONINFO osver = { sizeof(osver) };

    // Open the primary access token of the process for query and duplicate.
    if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY | TOKEN_DUPLICATE,
        &hToken))
    {
        dwError = GetLastError();
        goto Cleanup;
    }

    // Determine whether system is running Windows Vista or later operating
    // systems (major version >= 6) because they support linked tokens, but
    // previous versions (major version < 6) do not.
    if (!GetVersionEx(&osver))
    {
        dwError = GetLastError();
        goto Cleanup;
    }

    if (osver.dwMajorVersion >= 6)
    {
        // Running Windows Vista or later (major version >= 6).
        // Determine token type: limited, elevated, or default.
        TOKEN_ELEVATION_TYPE elevType;
        if (!GetTokenInformation(hToken, TokenElevationType, &elevType,
            sizeof(elevType), &cbSize))
        {
            dwError = GetLastError();
            goto Cleanup;
        }

        // If limited, get the linked elevated token for further check.
        if (TokenElevationTypeLimited == elevType)
        {
            if (!GetTokenInformation(hToken, TokenLinkedToken, &hTokenToCheck,
                sizeof(hTokenToCheck), &cbSize))
            {
                dwError = GetLastError();
                goto Cleanup;
            }
        }
    }

    // CheckTokenMembership requires an impersonation token. If we just got a
    // linked token, it already is an impersonation token.  If we did not get
    // a linked token, duplicate the original into an impersonation token for
    // CheckTokenMembership.
    if (!hTokenToCheck)
    {
        if (!DuplicateToken(hToken, SecurityIdentification, &hTokenToCheck))
        {
            dwError = GetLastError();
            goto Cleanup;
        }
    }

    // Create the SID corresponding to the Administrators group.
    BYTE adminSID[SECURITY_MAX_SID_SIZE];
    cbSize = sizeof(adminSID);
    if (!CreateWellKnownSid(WinBuiltinAdministratorsSid, NULL, &adminSID,
        &cbSize))
    {
        dwError = GetLastError();
        goto Cleanup;
    }

    // Check if the token to be checked contains admin SID.
    // http://msdn.microsoft.com/en-us/library/aa379596(VS.85).aspx:
    // To determine whether a SID is enabled in a token, that is, whether it
    // has the SE_GROUP_ENABLED attribute, call CheckTokenMembership.
    if (!CheckTokenMembership(hTokenToCheck, &adminSID, &fInAdminGroup))
    {
        dwError = GetLastError();
        goto Cleanup;
    }

Cleanup:
    // Centralized cleanup for all allocated resources.
    if (hToken)
    {
        CloseHandle(hToken);
        hToken = NULL;
    }
    if (hTokenToCheck)
    {
        CloseHandle(hTokenToCheck);
        hTokenToCheck = NULL;
    }

    // Throw the error if something failed in the function.
    if (ERROR_SUCCESS != dwError)
    {
        // throw dwError;
		return 2;
    }


    return fInAdminGroup;
}


DWORD IsRunAsAdmin()
{
    BOOL fIsRunAsAdmin = FALSE;
    DWORD dwError = ERROR_SUCCESS;
    PSID pAdministratorsGroup = NULL;

    // Allocate and initialize a SID of the administrators group.
    SID_IDENTIFIER_AUTHORITY NtAuthority = SECURITY_NT_AUTHORITY;
    if (!AllocateAndInitializeSid(
        &NtAuthority,
        2,
        SECURITY_BUILTIN_DOMAIN_RID,
        DOMAIN_ALIAS_RID_ADMINS,
        0, 0, 0, 0, 0, 0,
        &pAdministratorsGroup))
    {
        dwError = GetLastError();
        goto Cleanup;
    }

    // Determine whether the SID of administrators group is enabled in
    // the primary access token of the process.
    if (!CheckTokenMembership(NULL, pAdministratorsGroup, &fIsRunAsAdmin))
    {
        dwError = GetLastError();
        goto Cleanup;
    }

Cleanup:
    // Centralized cleanup for all allocated resources.
    if (pAdministratorsGroup)
    {
        FreeSid(pAdministratorsGroup);
        pAdministratorsGroup = NULL;
    }

    // Throw the error if something failed in the function.
    if (ERROR_SUCCESS != dwError)
    {
        // throw dwError;
		return 2;
    }

    return fIsRunAsAdmin;
}


BOOL IsProcessElevated()
{
    BOOL fIsElevated = FALSE;
    DWORD dwError = ERROR_SUCCESS;
    HANDLE hToken = NULL;

    // Open the primary access token of the process with TOKEN_QUERY.
    if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &hToken))
    {
        dwError = GetLastError();
        goto Cleanup;
    }

    // Retrieve token elevation information.
    TOKEN_ELEVATION elevation;
    DWORD dwSize;
    if (!GetTokenInformation(hToken, TokenElevation, &elevation,
        sizeof(elevation), &dwSize))
    {
        // When the process is run on operating systems prior to Windows
        // Vista, GetTokenInformation returns FALSE with the
        // ERROR_INVALID_PARAMETER error code because TokenElevation is
        // not supported on those operating systems.
        dwError = GetLastError();
        goto Cleanup;
    }

    fIsElevated = elevation.TokenIsElevated;

Cleanup:
    // Centralized cleanup for all allocated resources.
    if (hToken)
    {
        CloseHandle(hToken);
        hToken = NULL;
    }

    // Throw the error if something failed in the function.
    if (ERROR_SUCCESS != dwError)
    {
        // throw dwError;
		return 2;
    }

    return fIsElevated;
}

BOOL RelaunchAsAdmin() {
	wchar_t szPath[MAX_PATH];
	if (GetModuleFileNameW(NULL, szPath, ARRAYSIZE(szPath)))
	{
		// Launch itself as administrator.
		SHELLEXECUTEINFOW sei = { sizeof(sei) };
		sei.lpVerb = L"runas";
		sei.lpFile = szPath;
		sei.hwnd = NULL;
		sei.nShow = SW_NORMAL;

		if (!ShellExecuteExW(&sei))
		{
			DWORD dwError = GetLastError();
			if (dwError == ERROR_CANCELLED)
			{
				// The user refused the elevation.
                return 0;
			}
		}
		else
		{
			// EndDialog(NULL, TRUE);  // Quit itself
			return 1;
		}
	}
}
*/
import "C"

// IsUserInAdminGroup returns 0 if the user running Flying Carpet is not an administrator on the computer,
// 1 if the user is an administrator, or 2 if there was an error in the function.
func IsUserInAdminGroup() int {
	return int(C.IsUserInAdminGroup())
}

// IsRunAsAdmin returns 0 if Flying Carpet was not "Run as Administrator",
// 1 if it was, or 2 if there was an error in the function.
func IsRunAsAdmin() int {
	return int(C.IsRunAsAdmin())
}

// RelaunchAsAdmin relaunches Flying Carpet "as Administrator",
// and returns whether the user agreed to the UAC prompt.
func RelaunchAsAdmin() int {
	return int(C.RelaunchAsAdmin())
}
