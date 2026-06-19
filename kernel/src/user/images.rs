#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserlandImage {
    pub path: &'static str,
    pub name: &'static str,
    pub data: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/userland_images.rs"));

pub fn find(path: &str) -> Option<UserlandImage> {
    USERLAND_IMAGES
        .iter()
        .copied()
        .find(|image| image.path == path)
}

pub fn find_path_by_name(name: &str) -> &'static str {
    USERLAND_IMAGES
        .iter()
        .find(|image| image.name == name)
        .map(|image| image.path)
        .unwrap_or("")
}

pub fn normalize_path(path: &str) -> Option<&'static str> {
    if let Some(image) = find(path) {
        return Some(image.path);
    }

    if let Some(name) = path.strip_prefix("bin/") {
        return find(path).map(|image| image.path).or_else(|| {
            let normalized = find_path_by_name(name);
            (!normalized.is_empty()).then_some(normalized)
        });
    }

    let normalized = find_path_by_name(path);
    (!normalized.is_empty()).then_some(normalized)
}

pub fn default_image() -> Option<UserlandImage> {
    find("/bin/demo").or_else(|| USERLAND_IMAGES.first().copied())
}
