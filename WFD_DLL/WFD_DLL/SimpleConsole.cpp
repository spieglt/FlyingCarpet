//*********************************************************
//
// Copyright (c) Microsoft. All rights reserved.
// This code is licensed under the MIT License (MIT).
// THIS CODE IS PROVIDED *AS IS* WITHOUT WARRANTY OF
// ANY KIND, EITHER EXPRESS OR IMPLIED, INCLUDING ANY
// IMPLIED WARRANTIES OF FITNESS FOR A PARTICULAR
// PURPOSE, MERCHANTABILITY, OR NON-INFRINGEMENT.
//
//*********************************************************

#include "stdafx.h"
#include "SimpleConsole.h"
#include "WlanHostedNetworkWinRT.h"

SimpleConsole::SimpleConsole()
	: _apEvent(CreateEventEx(nullptr, nullptr, 0, WRITE_OWNER | EVENT_ALL_ACCESS))
{
	//std::wcout << L"In SimpleConsole constructor" << std::endl;
	HRESULT hr = _apEvent.IsValid() ? S_OK : HRESULT_FROM_WIN32(GetLastError());
	if (FAILED(hr))
	{
		//std::wcout << "Failed to create AP event: " << hr << std::endl;
		throw WlanHostedNetworkException("Create event failed", hr);
	}

	_hostedNetwork.RegisterListener(this);
	_hostedNetwork.RegisterPrompt(this);

	//std::wcout << L"Exiting SimpleConsole constructor" << std::endl;
}

SimpleConsole::~SimpleConsole()
{
	_hostedNetwork.RegisterListener(nullptr);
	_hostedNetwork.RegisterPrompt(nullptr);
}

void SimpleConsole::OnDeviceConnected(std::wstring remoteHostName)
{
	//std::wcout << std::endl << "Peer connected: " << remoteHostName << std::endl;
}

void SimpleConsole::OnDeviceDisconnected(std::wstring deviceId)
{
	//std::wcout << std::endl << "Peer disconnected: " << deviceId << std::endl;
}

void SimpleConsole::OnAdvertisementStarted()
{
	//std::wcout << "Soft AP started!" << std::endl
	//	<< "Peers can connect to: " << _hostedNetwork.GetSSID() << std::endl
	//	<< "Passphrase: " << _hostedNetwork.GetPassphrase() << std::endl;
	SetEvent(_apEvent.Get());
}

void SimpleConsole::OnAdvertisementStopped(std::wstring message)
{
	//std::wcout << "Soft AP stopped." << std::endl;
	SetEvent(_apEvent.Get());
}

void SimpleConsole::OnAdvertisementAborted(std::wstring message)
{
	//std::wcout << "Soft AP aborted: " << message << std::endl;
	SetEvent(_apEvent.Get());
}

void SimpleConsole::OnAsyncException(std::wstring message)
{
	//std::wcout << std::endl << "Caught exception in asynchronous method: " << message << std::endl;
}

void SimpleConsole::LogMessage(std::wstring message)
{
	//std::wcout << std::endl << message << std::endl;
}

bool SimpleConsole::AcceptIncommingConnection()
{
	//std::wcout << std::endl << "Accept peer connection? (y/n)" << std::endl;

	std::wstring response;
	getline(std::wcin, response);

	if (response.length() > 0 &&
		(response[0] == 'y' || response[0] == 'Y'))
	{
		return true;
	}

	return false;
}

bool SimpleConsole::ExecuteCommand(std::wstring command)
{
	// Simple command parsing logic

	if (command == L"quit" ||
		command == L"exit")
	{
		//std::wcout << std::endl << "Exiting" << std::endl;
		return false;
	}
	else if (command == L"start")
	{
		//std::wcout << std::endl << "Starting soft AP..." << std::endl;
		_hostedNetwork.Start();
		//std::wcout << std::endl << "Waiting for start event" << std::endl;
		WaitForSingleObjectEx(_apEvent.Get(), INFINITE, FALSE);
		//std::wcout << std::endl << "Received start event" << std::endl;
	}
	else if (command == L"stop")
	{
		//std::wcout << std::endl << "Stopping soft AP..." << std::endl;
		_hostedNetwork.Stop();
		//std::wcout << std::endl << "Waiting for stop event" << std::endl;
		WaitForSingleObjectEx(_apEvent.Get(), INFINITE, FALSE);
		//std::wcout << std::endl << "Received stop event" << std::endl;
	}
	else if (0 == command.compare(0, 4, L"ssid"))
	{
		// Parse the SSID as the first non-space character after ssid
		std::wstring ssid;
		std::wstring::size_type found = command.find_first_not_of(' ', 4);
		if (found != std::wstring::npos && found < command.length())
		{
			ssid = command.substr(found);
			//std::wcout << std::endl << "Setting SSID to " << ssid << std::endl;
			_hostedNetwork.SetSSID(ssid);
		}
		else
		{
			//std::wcout << std::endl << "Setting SSID FAILED, bad input" << std::endl;
		}
	}
	else if (0 == command.compare(0, 4, L"pass"))
	{
		// Parse the Passphrase as the first non-space character after pass
		std::wstring passphrase;
		std::wstring::size_type found = command.find_first_not_of(' ', 4);
		if (found != std::wstring::npos && found < command.length())
		{
			passphrase = command.substr(found);
			//std::wcout << std::endl << "Setting Passphrase to " << passphrase << std::endl;
			_hostedNetwork.SetPassphrase(passphrase);
		}
		else
		{
			//std::wcout << std::endl << "Setting Passphrase FAILED, bad input" << std::endl;
		}
	}
	else if (0 == command.compare(0, 10, L"autoaccept"))
	{
		std::wstring value;
		std::wstring::size_type found = command.find_first_not_of(' ', 10);
		if (found != std::wstring::npos && found < command.length())
		{
			value = command.substr(found);

			bool autoAccept = true;

			if (value == L"0")
			{
				autoAccept = false;
			}

			//std::wcout << std::endl << "Setting AutoAccept to " << autoAccept << " (input was " << value << ")" << std::endl;
			_hostedNetwork.SetAutoAccept(autoAccept);
		}
		else
		{
			//std::wcout << std::endl << "Setting AutoAccpet FAILED, bad input" << std::endl;
		}
	}
	else
	{
		//std::wcout << "Incorrect command input" << std::endl;
	}

	return true;
}
