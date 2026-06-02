use super::Version;
use std::str::FromStr;

pub const SUPPORTED_PLATFORM_APIS: &[&str] = &[
    "0.7", "0.8", "0.9", "0.10", "0.11", "0.12", "0.13", "0.14", "0.15",
];

pub const DEPRECATED_PLATFORM_APIS: &[&str] = &[];



pub fn is_supported(requested: &Version) -> bool {
    SUPPORTED_PLATFORM_APIS.iter().any(|&sup| {
        if let Ok(sup_ver) = Version::from_str(sup) {
            sup_ver.is_superset_of(requested)
        } else {
            false
        }
    })
}

pub fn is_deprecated(requested: &Version) -> bool {
    DEPRECATED_PLATFORM_APIS.iter().any(|&dep| {
        if let Ok(dep_ver) = Version::from_str(dep) {
            dep_ver.is_superset_of(requested)
        } else {
            false
        }
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum PlatformApiError {
    Empty,
    Invalid(String),
    Incompatible(String),
}

impl std::fmt::Display for PlatformApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlatformApiError::Empty => write!(f, "Platform API version is empty"),
            PlatformApiError::Invalid(v) => write!(f, "parse platform API '{}'", v),
            PlatformApiError::Incompatible(v) => write!(f, "platform API version '{}' is incompatible with the lifecycle", v),
        }
    }
}

pub fn verify_platform_api(requested_str: &str) -> Result<Version, PlatformApiError> {
    let clean = requested_str.trim();
    if clean.is_empty() {
        return Err(PlatformApiError::Empty);
    }

    let requested = Version::from_str(clean)
        .map_err(|_| PlatformApiError::Invalid(clean.to_string()))?;

    if is_supported(&requested) {
        if is_deprecated(&requested) {
            // Note: We can implement deprecation warnings if required
            eprintln!("Platform requested deprecated API '{}'", clean);
        }
        Ok(requested)
    } else {
        Err(PlatformApiError::Incompatible(clean.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_api_verification() {
        assert!(verify_platform_api("0.15").is_ok());
        assert!(verify_platform_api("0.7").is_ok());
        assert_eq!(verify_platform_api("0.6"), Err(PlatformApiError::Incompatible("0.6".to_string())));
        assert_eq!(verify_platform_api("bad-api"), Err(PlatformApiError::Invalid("bad-api".to_string())));
        assert_eq!(verify_platform_api(""), Err(PlatformApiError::Empty));
    }
}

