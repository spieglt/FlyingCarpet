use std::{net::AddrParseError, string::FromUtf8Error};

// TODO: use a macro or otherwise improve these

#[derive(Debug)]
pub struct FCError {
    pub message: String,
}

#[derive(Debug)]
pub enum DiscoveryError {
    NoNetworkInterface,
    MulticastBindFailed(String),
    HmacVerificationFailed,
    TimestampExpired,
    TimeoutWaitingForPeer,
}

impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiscoveryError::NoNetworkInterface => write!(f, "No usable network interface found"),
            DiscoveryError::MulticastBindFailed(msg) => {
                write!(f, "Failed to bind multicast socket: {}", msg)
            }
            DiscoveryError::HmacVerificationFailed => write!(f, "HMAC verification failed"),
            DiscoveryError::TimestampExpired => write!(f, "Timestamp expired"),
            DiscoveryError::TimeoutWaitingForPeer => write!(f, "Timeout waiting for peer"),
        }
    }
}

impl From<DiscoveryError> for FCError {
    fn from(value: DiscoveryError) -> Self {
        FCError {
            message: format!("Discovery error: {}", value),
        }
    }
}

impl std::fmt::Display for FCError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl From<std::io::Error> for FCError {
    fn from(value: std::io::Error) -> Self {
        FCError {
            message: format!("std::io::Error: {}", value),
        }
    }
}

impl From<AddrParseError> for FCError {
    fn from(value: AddrParseError) -> Self {
        FCError {
            message: format!("AddrParseError: {}", value),
        }
    }
}

impl From<std::sync::mpsc::RecvError> for FCError {
    fn from(value: std::sync::mpsc::RecvError) -> Self {
        FCError {
            message: format!("mpsc::RecvError: {}", value),
        }
    }
}

impl From<regex::Error> for FCError {
    fn from(value: regex::Error) -> Self {
        FCError {
            message: format!("Regex error: {}", value),
        }
    }
}

impl From<FromUtf8Error> for FCError {
    fn from(value: FromUtf8Error) -> Self {
        FCError {
            message: format!("FromUtf8 error: {}", value),
        }
    }
}

impl From<aes_gcm::Error> for FCError {
    fn from(value: aes_gcm::Error) -> Self {
        FCError {
            message: format!("AES-GCM error: {}", value),
        }
    }
}

impl From<std::path::StripPrefixError> for FCError {
    fn from(value: std::path::StripPrefixError) -> Self {
        FCError {
            message: format!("Strip prefix error: {}", value),
        }
    }
}

pub fn fc_error(message: &str) -> Result<(), FCError> {
    Err(FCError {
        message: message.to_string(),
    })
}
