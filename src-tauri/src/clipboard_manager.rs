//! Clipboard Manager Module
//! Handles clipboard monitoring, history storage, and paste injection

use arboard::{Clipboard, ImageData};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::{DateTime, Utc};
use image::{DynamicImage, ImageFormat};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use uuid::Uuid;

/// Maximum number of items to store in history
const MAX_HISTORY_SIZE: usize = 50;

/// Content type for clipboard items
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data")]
pub enum ClipboardContent {
    /// Plain text content
    Text(String),
    /// Image as base64 encoded PNG
    Image {
        base64: String,
        width: u32,
        height: u32,
    },
}

/// A single clipboard history item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardItem {
    /// Unique identifier
    pub id: String,
    /// The content
    pub content: ClipboardContent,
    /// When it was copied
    pub timestamp: DateTime<Utc>,
    /// Whether this item is pinned
    pub pinned: bool,
    /// Preview text (for display)
    pub preview: String,
}

impl ClipboardItem {
    /// Create a new text item
    pub fn new_text(text: String) -> Self {
        let preview = if text.len() > 100 {
            format!("{}...", &text[..100])
        } else {
            text.clone()
        };

        Self {
            id: Uuid::new_v4().to_string(),
            content: ClipboardContent::Text(text),
            timestamp: Utc::now(),
            pinned: false,
            preview,
        }
    }

    /// Create a new image item
    pub fn new_image(base64: String, width: u32, height: u32) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            content: ClipboardContent::Image {
                base64,
                width,
                height,
            },
            timestamp: Utc::now(),
            pinned: false,
            preview: format!("Image ({}x{})", width, height),
        }
    }

    /// Create a new image item with hash for deduplication
    pub fn new_image_with_hash(base64: String, width: u32, height: u32, hash: u64) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            content: ClipboardContent::Image {
                base64,
                width,
                height,
            },
            timestamp: Utc::now(),
            pinned: false,
            preview: format!("Image ({}x{}) #{}", width, height, hash),
        }
    }
}

/// Manages clipboard operations and history
pub struct ClipboardManager {
    history: Vec<ClipboardItem>,
    /// Track the last pasted content to avoid re-adding it to history
    last_pasted_text: Option<String>,
    last_pasted_image_hash: Option<u64>,
    /// Track last added text hash to prevent duplicates from rapid copies
    last_added_text_hash: Option<u64>,
}

impl ClipboardManager {
    /// Create a new clipboard manager
    pub fn new() -> Self {
        Self {
            history: Vec::with_capacity(MAX_HISTORY_SIZE),
            last_pasted_text: None,
            last_pasted_image_hash: None,
            last_added_text_hash: None,
        }
    }

    /// Get a clipboard instance (creates new each time for thread safety)
    fn get_clipboard() -> Result<Clipboard, arboard::Error> {
        Clipboard::new()
    }

    /// Get current text from clipboard
    pub fn get_current_text(&mut self) -> Result<String, arboard::Error> {
        Self::get_clipboard()?.get_text()
    }

    /// Get current image from clipboard with hash for change detection
    pub fn get_current_image(
        &mut self,
    ) -> Result<Option<(ImageData<'static>, u64)>, arboard::Error> {
        let mut clipboard = Self::get_clipboard()?;
        match clipboard.get_image() {
            Ok(image) => {
                // Create hash from image data for comparison
                let mut hasher = DefaultHasher::new();
                image.bytes.hash(&mut hasher);
                let hash = hasher.finish();

                // Convert to owned data
                let owned = ImageData {
                    width: image.width,
                    height: image.height,
                    bytes: image.bytes.into_owned().into(),
                };

                Ok(Some((owned, hash)))
            }
            Err(arboard::Error::ContentNotAvailable) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Add text to history
    pub fn add_text(&mut self, text: String) -> Option<ClipboardItem> {
        // Don't add empty strings
        if text.trim().is_empty() {
            return None;
        }

        // Compute hash for this text
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let text_hash = hasher.finish();

        // Skip if this is the same as the last added item (rapid copy detection)
        if Some(text_hash) == self.last_added_text_hash {
            return None;
        }

        // Skip if this was just pasted by us (avoid re-adding pasted content)
        if let Some(ref pasted) = self.last_pasted_text {
            if pasted == &text {
                // Clear it so future copies of same text are allowed
                self.last_pasted_text = None;
                return None;
            }
        }

        // Check if the first non-pinned item is the same text - skip if so
        let first_non_pinned = self.history.iter().find(|item| !item.pinned);
        if let Some(item) = first_non_pinned {
            if matches!(&item.content, ClipboardContent::Text(t) if t == &text) {
                // Same as the last item, don't add duplicate
                self.last_added_text_hash = Some(text_hash);
                return None;
            }
        }

        // Check for duplicates elsewhere in history (non-pinned items only)
        if let Some(pos) = self.history.iter().position(|item| {
            !item.pinned && matches!(&item.content, ClipboardContent::Text(t) if t == &text)
        }) {
            // Remove the duplicate so we can move it to top
            self.history.remove(pos);
        }

        // Update last added hash
        self.last_added_text_hash = Some(text_hash);

        let item = ClipboardItem::new_text(text);
        self.insert_item(item.clone());
        Some(item)
    }

    /// Add image to history
    pub fn add_image(&mut self, image_data: ImageData<'_>, hash: u64) -> Option<ClipboardItem> {
        // Skip if this was just pasted by us
        if let Some(pasted_hash) = self.last_pasted_image_hash {
            if pasted_hash == hash {
                self.last_pasted_image_hash = None;
                return None;
            }
        }

        // Check if the first non-pinned item is the same image (by hash stored in preview)
        let first_non_pinned = self.history.iter().find(|item| !item.pinned);
        if let Some(item) = first_non_pinned {
            if let ClipboardContent::Image { .. } = &item.content {
                // Check if hash matches (stored in the item)
                if item.preview.contains(&format!("#{}", hash)) {
                    return None;
                }
            }
        }

        // Convert to base64 PNG
        let img = DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(
                image_data.width as u32,
                image_data.height as u32,
                image_data.bytes.to_vec(),
            )
            .unwrap(),
        );

        let mut buffer = Cursor::new(Vec::new());
        if img.write_to(&mut buffer, ImageFormat::Png).is_err() {
            return None;
        }

        let base64 = BASE64.encode(buffer.get_ref());
        let item = ClipboardItem::new_image_with_hash(
            base64,
            image_data.width as u32,
            image_data.height as u32,
            hash,
        );

        self.insert_item(item.clone());
        Some(item)
    }

    /// Insert an item at the top of history (respecting pinned items)
    fn insert_item(&mut self, item: ClipboardItem) {
        // Find the first non-pinned position
        let insert_pos = self.history.iter().position(|i| !i.pinned).unwrap_or(0);
        self.history.insert(insert_pos, item);

        // Trim to max size (remove from end, but preserve pinned items)
        while self.history.len() > MAX_HISTORY_SIZE {
            if let Some(pos) = self.history.iter().rposition(|i| !i.pinned) {
                self.history.remove(pos);
            } else {
                break; // All items are pinned, don't remove any
            }
        }
    }

    /// Get the full history
    pub fn get_history(&self) -> Vec<ClipboardItem> {
        self.history.clone()
    }

    /// Get a specific item by ID
    pub fn get_item(&self, id: &str) -> Option<&ClipboardItem> {
        self.history.iter().find(|item| item.id == id)
    }

    /// Clear all non-pinned history
    pub fn clear(&mut self) {
        self.history.retain(|item| item.pinned);
    }

    /// Remove a specific item
    pub fn remove_item(&mut self, id: &str) {
        self.history.retain(|item| item.id != id);
    }

    /// Toggle pin status
    pub fn toggle_pin(&mut self, id: &str) -> Option<ClipboardItem> {
        if let Some(item) = self.history.iter_mut().find(|i| i.id == id) {
            item.pinned = !item.pinned;
            return Some(item.clone());
        }
        None
    }

    /// Mark content as pasted (to avoid re-adding it to history)
    pub fn mark_as_pasted(&mut self, item: &ClipboardItem) {
        match &item.content {
            ClipboardContent::Text(text) => {
                self.last_pasted_text = Some(text.clone());
                self.last_pasted_image_hash = None;
            }
            ClipboardContent::Image { .. } => {
                // Extract hash from preview
                if let Some(hash_str) = item.preview.split('#').nth(1) {
                    if let Ok(hash) = hash_str.parse::<u64>() {
                        self.last_pasted_image_hash = Some(hash);
                    }
                }
                self.last_pasted_text = None;
            }
        }
    }

    /// Paste an item (write to clipboard and simulate Ctrl+V)
    pub fn paste_item(&mut self, item: &ClipboardItem) -> Result<(), String> {
        // Mark as pasted BEFORE writing to clipboard to avoid duplicate detection
        self.mark_as_pasted(item);

        // Create a new clipboard instance for pasting
        let mut clipboard = Self::get_clipboard().map_err(|e| e.to_string())?;

        match &item.content {
            ClipboardContent::Text(text) => {
                clipboard.set_text(text).map_err(|e| e.to_string())?;
            }
            ClipboardContent::Image {
                base64,
                width,
                height,
            } => {
                let bytes = BASE64.decode(base64).map_err(|e| e.to_string())?;
                let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
                let rgba = img.to_rgba8();

                let image_data = ImageData {
                    width: *width as usize,
                    height: *height as usize,
                    bytes: rgba.into_raw().into(),
                };

                clipboard.set_image(image_data).map_err(|e| e.to_string())?;
            }
        }

        // Simulate Ctrl+V to paste
        simulate_paste()?;

        Ok(())
    }
}

/// Simulate Ctrl+V keypress for paste injection
#[cfg(target_os = "linux")]
fn simulate_paste() -> Result<(), String> {
    // Longer delay to ensure focus is properly restored and clipboard is ready
    std::thread::sleep(std::time::Duration::from_millis(10));

    eprintln!("[SimulatePaste] Sending Ctrl+V...");

    // Try uinput first - works for ALL apps (X11, XWayland, native Wayland)
    match simulate_paste_uinput() {
        Ok(()) => {
            eprintln!("[SimulatePaste] Ctrl+V sent via uinput");
            return Ok(());
        }
        Err(e) => {
            eprintln!("[SimulatePaste] uinput failed: {}, trying fallbacks...", e);
        }
    }

    // Fallback to enigo for XWayland apps
    match simulate_paste_enigo() {
        Ok(()) => {
            eprintln!("[SimulatePaste] Ctrl+V sent via enigo");
            return Ok(());
        }
        Err(e) => {
            eprintln!("[SimulatePaste] enigo failed: {}", e);
        }
    }

    // Last fallback to xdotool
    if std::env::var("DISPLAY").is_ok() {
        if let Ok(output) = std::process::Command::new("xdotool")
            .args(["key", "--clearmodifiers", "ctrl+v"])
            .output()
        {
            if output.status.success() {
                eprintln!("[SimulatePaste] Ctrl+V sent via xdotool");
                return Ok(());
            }
        }
    }

    Err("All paste methods failed".to_string())
}

/// Simulate paste using uinput (works for ALL apps including native Wayland)
#[cfg(target_os = "linux")]
fn simulate_paste_uinput() -> Result<(), String> {
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::os::unix::io::AsRawFd;

    // Linux input event codes
    const EV_SYN: u16 = 0x00;
    const EV_KEY: u16 = 0x01;
    const SYN_REPORT: u16 = 0x00;
    const KEY_LEFTCTRL: u16 = 29;
    const KEY_V: u16 = 47;

    // input_event struct layout for x86_64:
    // struct timeval { long tv_sec; long tv_usec; } = 16 bytes
    // __u16 type = 2 bytes
    // __u16 code = 2 bytes
    // __s32 value = 4 bytes
    // Total = 24 bytes

    fn make_event(type_: u16, code: u16, value: i32) -> [u8; 24] {
        let mut event = [0u8; 24];
        // timeval (16 bytes) - leave as zeros
        // type (2 bytes at offset 16)
        event[16..18].copy_from_slice(&type_.to_ne_bytes());
        // code (2 bytes at offset 18)
        event[18..20].copy_from_slice(&code.to_ne_bytes());
        // value (4 bytes at offset 20)
        event[20..24].copy_from_slice(&value.to_ne_bytes());
        event
    }

    // Open uinput device
    let mut uinput = OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .map_err(|e| format!("Failed to open /dev/uinput: {}", e))?;

    // Set up uinput device
    // UI_SET_EVBIT = 0x40045564
    // UI_SET_KEYBIT = 0x40045565
    const UI_SET_EVBIT: libc::c_ulong = 0x40045564;
    const UI_SET_KEYBIT: libc::c_ulong = 0x40045565;
    const UI_DEV_SETUP: libc::c_ulong = 0x405c5503;
    const UI_DEV_CREATE: libc::c_ulong = 0x5501;
    const UI_DEV_DESTROY: libc::c_ulong = 0x5502;

    unsafe {
        // Enable EV_KEY events
        if libc::ioctl(uinput.as_raw_fd(), UI_SET_EVBIT, EV_KEY as libc::c_int) < 0 {
            return Err("Failed to set EV_KEY".to_string());
        }

        // Enable the keys we need
        if libc::ioctl(
            uinput.as_raw_fd(),
            UI_SET_KEYBIT,
            KEY_LEFTCTRL as libc::c_int,
        ) < 0
        {
            return Err("Failed to set KEY_LEFTCTRL".to_string());
        }
        if libc::ioctl(uinput.as_raw_fd(), UI_SET_KEYBIT, KEY_V as libc::c_int) < 0 {
            return Err("Failed to set KEY_V".to_string());
        }

        // Setup device info
        #[repr(C)]
        struct UinputSetup {
            id: [u16; 4], // bus, vendor, product, version
            name: [u8; 80],
            ff_effects_max: u32,
        }

        let mut setup = UinputSetup {
            id: [0x03, 0x1234, 0x5678, 0x0001], // BUS_USB
            name: [0; 80],
            ff_effects_max: 0,
        };
        let name = b"clipboard-paste-helper";
        setup.name[..name.len()].copy_from_slice(name);

        if libc::ioctl(uinput.as_raw_fd(), UI_DEV_SETUP, &setup) < 0 {
            return Err("Failed to setup uinput device".to_string());
        }

        // Create the device
        if libc::ioctl(uinput.as_raw_fd(), UI_DEV_CREATE) < 0 {
            return Err("Failed to create uinput device".to_string());
        }
    }

    // Longer delay for device to be fully ready and recognized by the system
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Send Ctrl+V with proper timing
    // Press Ctrl first and wait for it to register
    uinput
        .write_all(&make_event(EV_KEY, KEY_LEFTCTRL, 1))
        .map_err(|e| e.to_string())?;
    uinput
        .write_all(&make_event(EV_SYN, SYN_REPORT, 0))
        .map_err(|e| e.to_string())?;
    uinput.flush().map_err(|e| e.to_string())?;

    // Wait for Ctrl to be fully registered
    std::thread::sleep(std::time::Duration::from_millis(30));

    // Press V while Ctrl is held
    uinput
        .write_all(&make_event(EV_KEY, KEY_V, 1))
        .map_err(|e| e.to_string())?;
    uinput
        .write_all(&make_event(EV_SYN, SYN_REPORT, 0))
        .map_err(|e| e.to_string())?;
    uinput.flush().map_err(|e| e.to_string())?;

    std::thread::sleep(std::time::Duration::from_millis(30));

    // Release V
    uinput
        .write_all(&make_event(EV_KEY, KEY_V, 0))
        .map_err(|e| e.to_string())?;
    uinput
        .write_all(&make_event(EV_SYN, SYN_REPORT, 0))
        .map_err(|e| e.to_string())?;
    uinput.flush().map_err(|e| e.to_string())?;

    std::thread::sleep(std::time::Duration::from_millis(30));

    // Release Ctrl last
    uinput
        .write_all(&make_event(EV_KEY, KEY_LEFTCTRL, 0))
        .map_err(|e| e.to_string())?;
    uinput
        .write_all(&make_event(EV_SYN, SYN_REPORT, 0))
        .map_err(|e| e.to_string())?;
    uinput.flush().map_err(|e| e.to_string())?;

    // Wait for events to be processed before destroying device
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Destroy the device
    unsafe {
        libc::ioctl(uinput.as_raw_fd(), UI_DEV_DESTROY);
    }

    Ok(())
}

/// Fallback paste simulation using enigo (X11/XWayland only)
#[cfg(target_os = "linux")]
fn simulate_paste_enigo() -> Result<(), String> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};

    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| {
        eprintln!("[SimulatePaste] Failed to create Enigo: {}", e);
        e.to_string()
    })?;

    // Press Ctrl
    enigo.key(Key::Control, Direction::Press).map_err(|e| {
        eprintln!("[SimulatePaste] Ctrl press failed: {}", e);
        e.to_string()
    })?;

    std::thread::sleep(std::time::Duration::from_millis(20));

    // Press and release V
    enigo
        .key(Key::Unicode('v'), Direction::Press)
        .map_err(|e| {
            eprintln!("[SimulatePaste] V press failed: {}", e);
            e.to_string()
        })?;

    std::thread::sleep(std::time::Duration::from_millis(20));

    enigo
        .key(Key::Unicode('v'), Direction::Release)
        .map_err(|e| {
            eprintln!("[SimulatePaste] V release failed: {}", e);
            e.to_string()
        })?;

    std::thread::sleep(std::time::Duration::from_millis(20));

    // Release Ctrl
    enigo.key(Key::Control, Direction::Release).map_err(|e| {
        eprintln!("[SimulatePaste] Ctrl release failed: {}", e);
        e.to_string()
    })?;

    eprintln!("[SimulatePaste] Ctrl+V sent via enigo");
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn simulate_paste() -> Result<(), String> {
    // Fallback for other platforms - just set clipboard
    Ok(())
}

impl Default for ClipboardManager {
    fn default() -> Self {
        Self::new()
    }
}
