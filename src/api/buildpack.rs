use super::Version;
use std::str::FromStr;

pub const SUPPORTED_BUILDPACK_APIS: &[&str] = &[
    "0.7", "0.8", "0.9", "0.10", "0.11", "0.12",
];

pub const DEPRECATED_BUILDPACK_APIS: &[&str] = &[];



pub fn is_supported(requested: &Version) -> bool {
    SUPPORTED_BUILDPACK_APIS.iter().any(|&sup| {
        if let Ok(sup_ver) = Version::from_str(sup) {
            sup_ver.is_superset_of(requested)
        } else {
            false
        }
    })
}

pub fn is_deprecated(requested: &Version) -> bool {
    DEPRECATED_BUILDPACK_APIS.iter().any(|&dep| {
        if let Ok(dep_ver) = Version::from_str(dep) {
            dep_ver.is_superset_of(requested)
        } else {
            false
        }
    })
}

pub fn verify_buildpack_api(bp_id: &str, requested_str: &str) -> Result<Version, String> {
    let clean = requested_str.trim();
    let requested = Version::from_str(clean)
        .map_err(|e| format!("Parse buildpack API '{}' for buildpack '{}': {}", clean, bp_id, e))?;

    if is_supported(&requested) {
        if is_deprecated(&requested) {
            eprintln!("Buildpack '{}' requested deprecated API '{}'", bp_id, clean);
        }
        Ok(requested)
    } else {
        Err(format!(
            "buildpack API version '{}' is incompatible with the lifecycle for buildpack '{}'",
            clean, bp_id
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buildpack_api_verification() {
        assert!(verify_buildpack_api("my-bp", "0.12").is_ok());
        assert!(verify_buildpack_api("my-bp", "0.7").is_ok());
        assert!(verify_buildpack_api("my-bp", "0.6").is_err());
    }
}
