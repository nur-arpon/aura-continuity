//! UI Automation reads: browser address bar, YouTube seek position, selected text.
//! Call only from a thread that has run `com_init()`.

use windows::core::{Interface, BSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::*;

/// Initialise COM (STA) for the current thread. STA, not MTA: Explorer's shell objects
/// fail cross-apartment calls from an MTA with RPC_E_CANTCALLOUT_ININPUTSYNCCALL
/// (0x8001010D, observed on this PC). Capture runs on its own short-lived thread.
pub fn com_init() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

pub struct Uia {
    a: IUIAutomation,
}

/// Chromium's accessible name for the omnibox (English UI).
const OMNIBOX_CLASS: &str = "OmniboxViewViews";
const OMNIBOX_NAMES: &[&str] = &["Address and search bar", "Search or enter address", "Address bar"];

impl Uia {
    pub fn new() -> Result<Self, String> {
        let a: IUIAutomation = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }
            .map_err(|e| format!("UI Automation unavailable: {e}"))?;
        Ok(Self { a })
    }

    fn type_cond(&self, t: UIA_CONTROLTYPE_ID) -> Option<IUIAutomationCondition> {
        unsafe { self.a.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(t.0)).ok() }
    }

    fn name_cond(&self, name: &str) -> Option<IUIAutomationCondition> {
        unsafe { self.a.CreatePropertyCondition(UIA_NamePropertyId, &VARIANT::from(BSTR::from(name))).ok() }
    }

    fn class_cond(&self, class: &str) -> Option<IUIAutomationCondition> {
        unsafe { self.a.CreatePropertyCondition(UIA_ClassNamePropertyId, &VARIANT::from(BSTR::from(class))).ok() }
    }

    fn and(&self, a: &IUIAutomationCondition, b: &IUIAutomationCondition) -> Option<IUIAutomationCondition> {
        unsafe { self.a.CreateAndCondition(a, b).ok() }
    }

    fn find_first(&self, root: &IUIAutomationElement, c: &IUIAutomationCondition) -> Option<IUIAutomationElement> {
        unsafe { root.FindFirst(TreeScope_Descendants, c).ok() }.filter(|e| !e.as_raw().is_null())
    }

    /// Text in a Chromium-family browser's address bar (as displayed - often without "https://").
    pub fn browser_address(&self, hwnd: HWND) -> Result<String, String> {
        let root = unsafe { self.a.ElementFromHandle(hwnd) }.map_err(|e| format!("ElementFromHandle: {e}"))?;
        let edit = self.type_cond(UIA_EditControlTypeId).ok_or("condition")?;
        let mut el = None;
        // Locale- and profile-independent: every Chromium window (Brave/Chrome/Edge, any profile,
        // incognito, guest) names its address field's class "OmniboxViewViews".
        if let Some(c) = self.class_cond(OMNIBOX_CLASS).and_then(|cc| self.and(&edit, &cc)) {
            el = self.find_first(&root, &c);
        }
        for n in OMNIBOX_NAMES {
            if el.is_some() {
                break;
            }
            if let Some(c) = self.name_cond(n).and_then(|nc| self.and(&edit, &nc)) {
                el = self.find_first(&root, &c);
                if el.is_some() {
                    break;
                }
            }
        }
        // Non-English UI: the first Edit in tree order is the omnibox (the toolbar comes
        // before the page content).
        let el = el.or_else(|| self.find_first(&root, &edit)).ok_or("address bar not found")?;
        let vp: IUIAutomationValuePattern =
            unsafe { el.GetCurrentPatternAs(UIA_ValuePatternId) }.map_err(|e| format!("no ValuePattern: {e}"))?;
        let v = unsafe { vp.CurrentValue() }.map_err(|e| format!("CurrentValue: {e}"))?;
        Ok(v.to_string())
    }

    /// Current YouTube playback position in seconds, read from the player's "Seek slider"
    /// (aria-valuenow = seconds). Best effort: None if the slider is not exposed.
    pub fn youtube_position(&self, hwnd: HWND) -> Option<u64> {
        let root = unsafe { self.a.ElementFromHandle(hwnd) }.ok()?;
        let slider = self.type_cond(UIA_SliderControlTypeId)?;
        let c = self.and(&slider, &self.name_cond("Seek slider")?)?;
        let Some(el) = self.find_first(&root, &c) else {
            return self.youtube_clock_text(&root);
        };
        let Some(rv) = unsafe { el.GetCurrentPatternAs::<IUIAutomationRangeValuePattern>(UIA_RangeValuePatternId) }.ok() else {
            return self.youtube_clock_text(&root);
        };
        let v = unsafe { rv.CurrentValue() }.ok()?;
        if v.is_finite() && v >= 0.0 {
            return Some(v as u64);
        }
        // Fallback: the value text, e.g. "4:05" style.
        let vp: IUIAutomationValuePattern = unsafe { el.GetCurrentPatternAs(UIA_ValuePatternId) }.ok()?;
        let s = unsafe { vp.CurrentValue() }.ok()?.to_string();
        crate::core::youtube::parse_clock(&s)
    }

    /// Fallback: the player's "1:23 / 10:00" time display, a Text element. Only the first
    /// match whose text parses as a clock pair is used.
    fn youtube_clock_text(&self, root: &IUIAutomationElement) -> Option<u64> {
        let c = self.type_cond(UIA_TextControlTypeId)?;
        let all = unsafe { root.FindAll(TreeScope_Descendants, &c) }.ok()?;
        for i in 0..unsafe { all.Length() }.ok()? {
            let Ok(e) = (unsafe { all.GetElement(i) }) else { continue };
            let n = unsafe { e.CurrentName() }.map(|b| b.to_string()).unwrap_or_default();
            if n.contains(" / ") {
                if let Some(t) = crate::core::youtube::parse_clock(&n) {
                    return Some(t);
                }
            }
        }
        None
    }

    /// Diagnostics: names of buttons in the window whose name contains `needle`
    /// (e.g. "(k)" finds YouTube's Play/Pause button and tells whether it is playing).
    pub fn button_names(&self, hwnd: HWND, needle: &str) -> Vec<String> {
        let mut out = Vec::new();
        unsafe {
            let Ok(root) = self.a.ElementFromHandle(hwnd) else { return out };
            let Some(c) = self.type_cond(UIA_ButtonControlTypeId) else { return out };
            let Ok(all) = root.FindAll(TreeScope_Descendants, &c) else { return out };
            for i in 0..all.Length().unwrap_or(0) {
                if let Ok(e) = all.GetElement(i) {
                    let n = e.CurrentName().map(|b| b.to_string()).unwrap_or_default();
                    if n.contains(needle) {
                        out.push(n);
                    }
                }
            }
        }
        out
    }

    /// Names of the tab strip's tabs (titles only), in order. Empty if the strip is not exposed.
    pub fn tab_titles(&self, hwnd: HWND) -> Vec<String> {
        let mut out = Vec::new();
        unsafe {
            let Ok(root) = self.a.ElementFromHandle(hwnd) else { return out };
            let Some(c) = self.type_cond(UIA_TabItemControlTypeId) else { return out };
            let Ok(all) = root.FindAll(TreeScope_Descendants, &c) else { return out };
            for i in 0..all.Length().unwrap_or(0) {
                if let Ok(e) = all.GetElement(i) {
                    let n = e.CurrentName().map(|b| b.to_string()).unwrap_or_default();
                    if !n.trim().is_empty() {
                        out.push(n);
                    }
                }
            }
        }
        out
    }

    /// Tab strip with selection state: (name, is the selected tab), in order. Empty if the strip is not exposed.
    pub fn tabs(&self, hwnd: HWND) -> Vec<(String, bool)> {
        let mut out = Vec::new();
        unsafe {
            let Ok(root) = self.a.ElementFromHandle(hwnd) else { return out };
            let Some(c) = self.type_cond(UIA_TabItemControlTypeId) else { return out };
            // web pages expose their chips / tabs as TabItems too: only the browser's own strip counts
            let wr = root.CurrentBoundingRectangle().unwrap_or_default();
            let limit = wr.top + ((wr.bottom - wr.top) / 8).max(96);
            let Ok(all) = root.FindAll(TreeScope_Descendants, &c) else { return out };
            for i in 0..all.Length().unwrap_or(0) {
                if let Ok(e) = all.GetElement(i) {
                    let n = e.CurrentName().map(|b| b.to_string()).unwrap_or_default();
                    if n.trim().is_empty() {
                        continue;
                    }
                    // Chromium names its strip tabs "Title - Memory usage - 120 MB"; failing that, the strip sits at the very top
                    let strip = n.to_ascii_lowercase().contains("memory usage") || e.CurrentBoundingRectangle().map(|r| r.top <= limit).unwrap_or(true);
                    if !strip {
                        continue;
                    }
                    let sel = e
                        .GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
                        .ok()
                        .and_then(|p| p.CurrentIsSelected().ok())
                        .map(|b| b.as_bool())
                        .unwrap_or(false);
                    out.push((n, sel));
                }
            }
        }
        out
    }

    /// Item 19: bring the tab whose title matches `title` to the front of its window (SelectionItem
    /// pattern, the same as a click on the tab). True when a tab was selected.
    pub fn select_tab(&self, hwnd: HWND, title: &str) -> bool {
        unsafe {
            let Ok(root) = self.a.ElementFromHandle(hwnd) else { return false };
            let Some(c) = self.type_cond(UIA_TabItemControlTypeId) else { return false };
            let Ok(all) = root.FindAll(TreeScope_Descendants, &c) else { return false };
            for i in 0..all.Length().unwrap_or(0) {
                let Ok(e) = all.GetElement(i) else { continue };
                let n = e.CurrentName().map(|b| b.to_string()).unwrap_or_default();
                if crate::core::recent::title_match(&n, title) {
                    if let Ok(sp) = e.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId) {
                        return sp.Select().is_ok();
                    }
                }
            }
        }
        false
    }

    /// Selected text in the focused control, if it supports the Text pattern.
    pub fn selected_text(&self) -> Option<String> {
        unsafe {
            let f = self.a.GetFocusedElement().ok()?;
            let tp: IUIAutomationTextPattern = f.GetCurrentPatternAs(UIA_TextPatternId).ok()?;
            let ranges = tp.GetSelection().ok()?;
            let mut out = String::new();
            for i in 0..ranges.Length().ok()? {
                if let Ok(r) = ranges.GetElement(i) {
                    if let Ok(t) = r.GetText(-1) {
                        out.push_str(&t.to_string());
                    }
                }
            }
            let t = out.trim().to_string();
            (!t.is_empty()).then_some(t)
        }
    }
}
