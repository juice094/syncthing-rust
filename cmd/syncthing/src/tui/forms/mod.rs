//! Unified form system shared by the add/edit device/folder popups.
//!
//! The `FormField::editable` flag distinguishes add mode (all fields
//! editable) from edit mode (ID field read-only), removing most of the
//! former per-popup duplication.

/// Form key handling.
pub mod handler;
/// Form rendering.
pub mod render;

/// One form field definition.
pub struct FormField {
    pub name: &'static str,
    pub label: &'static str,
    pub value: String,
    /// true in add mode (editable), false in edit mode (read-only).
    pub editable: bool,
    pub hint: Option<&'static str>,
}

/// Unified form state.
pub struct FormState {
    pub fields: Vec<FormField>,
    pub focus: usize,
    pub title: &'static str,
    pub width: u16,
    pub height: u16,
    pub error: Option<String>,
}

/// Action the caller should take after a key press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormAction {
    Continue,
    Submit,
    Cancel,
}

impl FormState {
    /// Creates an empty form with the given title and popup size.
    pub fn new(title: &'static str, width: u16, height: u16) -> Self {
        Self {
            fields: Vec::new(),
            focus: 0,
            title,
            width,
            height,
            error: None,
        }
    }

    /// Appends a field (builder style).
    pub fn add_field(
        mut self,
        name: &'static str,
        label: &'static str,
        value: String,
        editable: bool,
        hint: Option<&'static str>,
    ) -> Self {
        self.fields.push(FormField {
            name,
            label,
            value,
            editable,
            hint,
        });
        self
    }

    /// Returns the value of the field named `name`.
    pub fn value(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|f| f.name == name)
            .map(|f| f.value.as_str())
    }

    /// Number of text fields (excludes extended selections like the device list).
    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    /// Sets the error line shown under the form.
    pub fn set_error(&mut self, msg: String) {
        self.error = Some(msg);
    }

    /// Clears the form error line.
    pub fn clear_error(&mut self) {
        self.error = None;
    }

    /// Returns the name of the focused field, if any.
    pub fn focused_field_name(&self) -> Option<&'static str> {
        self.fields.get(self.focus).map(|f| f.name)
    }

    /// Formats raw Device ID input as 8 groups of 7 alphanumeric chars joined with `-`.
    pub fn format_device_id(input: &str) -> String {
        let cleaned: Vec<char> = input
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        let groups: Vec<String> = cleaned
            .chunks(7)
            .take(8)
            .map(|chunk| chunk.iter().collect())
            .collect();
        groups.join("-")
    }

    /// Appends one character to the focused field (with Device ID grouping).
    pub fn append_char(&mut self, c: char) {
        if self.focused_field_name() == Some("device_id") {
            if !c.is_ascii_alphanumeric() {
                return;
            }
            if let Some(field) = self.fields.get_mut(self.focus) {
                if field.editable {
                    field.value.push(c);
                    field.value = Self::format_device_id(&field.value);
                }
            }
        } else if let Some(field) = self.fields.get_mut(self.focus) {
            if field.editable {
                field.value.push(c);
            }
        }
    }

    /// Deletes one character from the focused field (with Device ID grouping).
    pub fn backspace(&mut self) {
        if self.focused_field_name() == Some("device_id") {
            if let Some(field) = self.fields.get_mut(self.focus) {
                if field.editable {
                    let cleaned: String = field
                        .value
                        .chars()
                        .filter(|c| c.is_ascii_alphanumeric())
                        .collect();
                    let trimmed: String = cleaned
                        .chars()
                        .take(cleaned.len().saturating_sub(1))
                        .collect();
                    field.value = Self::format_device_id(&trimmed);
                }
            }
        } else if let Some(field) = self.fields.get_mut(self.focus) {
            if field.editable {
                field.value.pop();
            }
        }
    }

    /// Moves focus to the next field; optionally skips read-only fields.
    pub fn focus_next(&mut self, skip_readonly: bool) {
        let count = self.fields.len();
        if count == 0 {
            return;
        }
        let start = self.focus;
        loop {
            self.focus = (self.focus + 1) % count;
            if !skip_readonly || self.fields[self.focus].editable || self.focus == start {
                break;
            }
        }
    }

    /// Moves focus to the previous field; optionally skips read-only fields.
    pub fn focus_prev(&mut self, skip_readonly: bool) {
        let count = self.fields.len();
        if count == 0 {
            return;
        }
        let start = self.focus;
        loop {
            if self.focus == 0 {
                self.focus = count - 1;
            } else {
                self.focus -= 1;
            }
            if !skip_readonly || self.fields[self.focus].editable || self.focus == start {
                break;
            }
        }
    }

    /// Whether focus is on the extended selection list (past the text fields).
    pub fn is_on_list(&self) -> bool {
        self.focus == self.fields.len()
    }
}
