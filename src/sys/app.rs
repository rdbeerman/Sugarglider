// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Interfaces to macOS APIs for interacting with other applications.

use std::ffi::c_int;
use std::fmt::{Debug, Formatter};
use std::io;
use std::ptr::NonNull;

use accessibility::{AXAttribute, AXAttributeValue, AXError, AXUIElement, AXUIElementAttributes};
pub use accessibility_sys::pid_t;
use accessibility_sys::{
    kAXFocusedApplicationAttribute, kAXFocusedUIElementAttribute, kAXStandardWindowSubrole,
    kAXWindowRole,
};
use objc2::rc::Retained;
use objc2::{class, msg_send};
use objc2_app_kit::{NSRunningApplication, NSWorkspace};
use objc2_core_foundation::{CFBoolean, CFRetained, CFString, CFType, CGRect};
use objc2_foundation::NSString;
use redact::Secret;
use serde::{Deserialize, Serialize};

use super::geometry::CGRectDef;
use super::window_server::WindowServerId;

pub fn running_apps(bundle: Option<String>) -> impl Iterator<Item = (pid_t, AppInfo)> {
    NSWorkspace::sharedWorkspace()
        .runningApplications()
        .into_iter()
        .flat_map(move |app| {
            let bundle_id = app.bundle_id()?.to_string();
            if let Some(filter) = &bundle {
                if !bundle_id.contains(filter) {
                    return None;
                }
            }
            Some((app.pid(), AppInfo::from(&*app)))
        })
}

pub trait NSRunningApplicationExt {
    fn with_process_id(pid: pid_t) -> Option<Retained<Self>>;
    fn pid(&self) -> pid_t;
    fn bundle_id(&self) -> Option<Retained<NSString>>;
    fn localized_name(&self) -> Option<Retained<NSString>>;
}

impl NSRunningApplicationExt for NSRunningApplication {
    fn with_process_id(pid: pid_t) -> Option<Retained<Self>> {
        unsafe {
            // For some reason this binding isn't generated in icrate.
            msg_send![class!(NSRunningApplication), runningApplicationWithProcessIdentifier:pid]
        }
    }
    fn pid(&self) -> pid_t {
        unsafe { msg_send![self, processIdentifier] }
    }
    fn bundle_id(&self) -> Option<Retained<NSString>> {
        self.bundleIdentifier()
    }
    fn localized_name(&self) -> Option<Retained<NSString>> {
        self.localizedName()
    }
}

#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub struct AppInfo {
    pub bundle_id: Option<String>,
    pub localized_name: Option<String>,
}

impl From<&NSRunningApplication> for AppInfo {
    fn from(app: &NSRunningApplication) -> Self {
        AppInfo {
            bundle_id: app.bundle_id().as_deref().map(ToString::to_string),
            localized_name: app.localized_name().as_deref().map(ToString::to_string),
        }
    }
}

/// The process that has a process id.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Process {
    /// No process has the id.
    Gone,
    /// A process has the id. `bundle_id` is the bundle id of its app, if it is
    /// an app that has one.
    Running { bundle_id: Option<String> },
}

impl Process {
    pub fn with_pid(pid: pid_t) -> Process {
        if !process_exists(pid) {
            return Process::Gone;
        }
        let bundle_id = NSRunningApplication::with_process_id(pid)
            .and_then(|app| app.bundle_id())
            .map(|id| id.to_string());
        Process::Running { bundle_id }
    }
}

/// The error `kill` reports when no process has the id.
const ESRCH: c_int = 3;

fn process_exists(pid: pid_t) -> bool {
    // Zero and negative ids name process groups, not one process.
    if pid <= 0 {
        return false;
    }
    // Signal 0 checks for the process without sending it anything.
    if unsafe { kill(pid, 0) } == 0 {
        return true;
    }
    // Any other error, such as EPERM, means the process exists.
    io::Error::last_os_error().raw_os_error() != Some(ESRCH)
}

unsafe extern "C" {
    fn kill(pid: pid_t, signal: c_int) -> c_int;
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WindowInfo {
    pub is_standard: bool,
    // This only gets used for the record/replay feature.
    #[serde(serialize_with = "redact::expose_secret")]
    pub title: Secret<String>,
    #[serde(with = "CGRectDef")]
    pub frame: CGRect,
    pub sys_id: Option<WindowServerId>,
    pub is_resizable: bool,
    /// The macOS Accessibility AXRole, e.g. "AXWindow".
    pub ax_role: String,
    /// The macOS Accessibility AXSubrole, e.g. "AXStandardWindow".
    pub ax_subrole: Option<String>,
}

impl TryFrom<&AXUIElement> for WindowInfo {
    type Error = accessibility::Error;
    fn try_from(element: &AXUIElement) -> Result<Self, accessibility::Error> {
        let role = element.role()?;
        let subrole = match element.subrole() {
            Ok(s) => Some(s),
            Err(accessibility::Error::Ax(e))
                if e == AXError::NoValue || e == AXError::AttributeUnsupported =>
            {
                None
            }
            Err(e) => return Err(e),
        };
        let is_standard = role.to_string() == kAXWindowRole
            && subrole.as_ref().is_some_and(|s| s.to_string() == kAXStandardWindowSubrole);
        let ax_subrole = subrole.map(|s| s.to_string());
        Ok(WindowInfo {
            is_standard,
            title: element.title().map(|t| t.to_string().into()).unwrap_or_default(),
            frame: element.frame()?,
            sys_id: WindowServerId::try_from(element).ok(),
            is_resizable: element.is_settable(&AXAttribute::size())?,
            ax_role: role.to_string(),
            ax_subrole,
        })
    }
}

pub trait AXUIElementExt {
    /// "Enhanced user interface" mode for screen readers and other accessibility apps.
    ///
    /// For most apps this is disabled unless a screen reader is running.
    /// One exception is Firefox, which seems to enable it automatically as soon
    /// as we do anything with accessibility
    /// (https://bugzilla.mozilla.org/show_bug.cgi?id=1845364).
    ///
    /// It seems to do two things:
    ///
    /// * Allows full access to UI elements through the API.
    /// * Animates window moves and resizes. These interfere with Glide animations.
    ///
    /// Other window managers disable this before moving or resizing a window;
    /// see https://issues.chromium.org/issues/40865608.
    fn enhanced_user_interface(&self) -> Result<bool, accessibility::Error>;
    fn set_enhanced_user_interface(&self, enabled: bool) -> Result<(), accessibility::Error>;

    /// The process the element belongs to.
    fn pid(&self) -> Result<pid_t, accessibility::Error>;

    /// The application with keyboard focus, which is only available on the
    /// system-wide element.
    ///
    /// This follows keyboard focus and not the frontmost application, so it
    /// points at a non-activating panel like Spotlight's while one is open.
    fn focused_application(&self) -> Result<CFRetained<AXUIElement>, accessibility::Error>;

    /// The element with keyboard focus, which is only available on the
    /// system-wide element.
    fn focused_ui_element(&self) -> Result<CFRetained<AXUIElement>, accessibility::Error>;

    fn privacy_sensitive_inspect(&self) -> Inspect<'_>;
}

impl AXUIElementExt for AXUIElement {
    fn enhanced_user_interface(&self) -> Result<bool, accessibility::Error> {
        Ok(self.attribute(&enhanced_ui())?.downcast::<CFBoolean>().is_ok_and(|b| b.value()))
    }
    fn set_enhanced_user_interface(&self, enabled: bool) -> Result<(), accessibility::Error> {
        self.set_attribute(&enhanced_ui(), CFBoolean::new(enabled))
    }

    fn pid(&self) -> Result<pid_t, accessibility::Error> {
        let mut pid = 0;
        // SAFETY: The out parameter is a valid pointer.
        let res = unsafe { self.as_sys().pid(NonNull::from(&mut pid)) };
        if let Some(err) = AXError::from_raw(res) {
            return Err(accessibility::Error::Ax(err));
        }
        Ok(pid)
    }

    fn focused_application(&self) -> Result<CFRetained<AXUIElement>, accessibility::Error> {
        AXUIElement::downcast(
            self.attribute(&system_wide_attribute(kAXFocusedApplicationAttribute))?,
        )
    }

    fn focused_ui_element(&self) -> Result<CFRetained<AXUIElement>, accessibility::Error> {
        AXUIElement::downcast(self.attribute(&system_wide_attribute(kAXFocusedUIElementAttribute))?)
    }

    fn privacy_sensitive_inspect(&self) -> Inspect<'_> {
        Inspect(self)
    }
}

fn enhanced_ui() -> AXAttribute<CFType> {
    AXAttribute::new(&CFString::from_static_str("AXEnhancedUserInterface"))
}

fn system_wide_attribute(name: &'static str) -> AXAttribute<CFType> {
    AXAttribute::new(&CFString::from_static_str(name))
}

pub struct Inspect<'a>(&'a AXUIElement);

impl Debug for Inspect<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        let mut st = f.debug_struct("AXWindow");
        for attr in self.0.attribute_names().unwrap().iter() {
            if let Ok(value) = self.0.attribute(&AXAttribute::new(&attr)) {
                st.field(&attr.to_string(), &*value);
            }
        }
        st.finish()
    }
}

pub struct ProcessInfo {
    pub is_xpc: bool,
}

impl ProcessInfo {
    pub fn for_pid(pid: pid_t) -> Result<Self, ()> {
        let psn = ProcessSerialNumber::for_pid(pid)?;

        let mut info = ProcessInfoRec::default();
        info.processInfoLength = size_of::<ProcessInfoRec>() as _;
        if unsafe { GetProcessInformation(&psn, &mut info) } != 0 {
            return Err(());
        }

        Ok(Self {
            is_xpc: info.processType.to_be_bytes() == *b"XPC!",
        })
    }
}

type FourCharCode = u32;
type OSType = FourCharCode;

#[allow(dead_code)]
#[allow(non_snake_case)]
#[repr(C, packed(2))]
#[derive(Default)]
struct ProcessInfoRec {
    processInfoLength: u32,
    processName: *const u8,
    processNumber: ProcessSerialNumber,
    processType: u32,
    processSignature: OSType,
    processMode: u32,
    processLocation: *const u8,
    processSize: u32,
    processFreeMem: u32,
    processLauncher: ProcessSerialNumber,
    processLaunchDate: u32,
    processActiveTime: u32,
    processAppRef: *const u8,
}
const _: () = if size_of::<ProcessInfoRec>() != 72 {
    panic!("unexpected size")
};

#[repr(C)]
#[derive(Default)]
pub(super) struct ProcessSerialNumber {
    high: u32,
    low: u32,
}

impl ProcessSerialNumber {
    pub(super) fn for_pid(pid: pid_t) -> Result<Self, ()> {
        let mut psn = ProcessSerialNumber::default();
        if unsafe { GetProcessForPID(pid, &mut psn) } == 0 {
            Ok(psn)
        } else {
            Err(())
        }
    }

    pub(super) fn pid(&self) -> Result<pid_t, ()> {
        let mut pid = 0;
        if unsafe { GetProcessPID(self, &mut pid) } == 0 {
            Ok(pid)
        } else {
            Err(())
        }
    }
}

type OSErr = i16;
type OSStatus = i32;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    // Deprecated in macOS 10.9.
    fn GetProcessForPID(pid: pid_t, psn: *mut ProcessSerialNumber) -> OSStatus;

    // Deprecated in macOS 10.9.
    fn GetProcessPID(psn: *const ProcessSerialNumber, pid: *mut pid_t) -> OSStatus;

    // Deprecated in macOS 10.9.
    fn GetProcessInformation(psn: *const ProcessSerialNumber, info: *mut ProcessInfoRec) -> OSErr;
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::{Process, pid_t, process_exists};

    #[test]
    fn this_process_exists() {
        assert!(process_exists(std::process::id() as pid_t));
        assert!(matches!(
            Process::with_pid(std::process::id() as pid_t),
            Process::Running { .. }
        ));
    }

    #[test]
    fn a_process_that_ended_is_gone() {
        let mut child = Command::new("/usr/bin/true").spawn().unwrap();
        let pid = child.id() as pid_t;
        child.wait().unwrap();
        assert_eq!(Process::Gone, Process::with_pid(pid));
    }

    #[test]
    fn ids_of_process_groups_are_no_process() {
        assert!(!process_exists(0));
        assert!(!process_exists(-1));
    }
}
