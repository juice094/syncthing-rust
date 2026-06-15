//! 统一表单系统 — 替代 4 处重复的表单渲染和按键处理代码。
//!
//! 通过 `FormField` 的 `editable` 标志区分 add 模式（全部可编辑）与 edit 模式（ID 字段只读），
//! 消除 add_device/add_folder/edit_device/edit_folder 的 70-80% 重复。

pub mod handler;
pub mod render;

/// 表单字段定义
pub struct FormField {
    pub name: &'static str,
    pub label: &'static str,
    pub value: String,
    /// true = add 模式（可编辑），false = edit 模式（只读）
    pub editable: bool,
    pub hint: Option<&'static str>,
}

/// 统一表单状态
pub struct FormState {
    pub fields: Vec<FormField>,
    pub focus: usize,
    pub title: &'static str,
    pub width: u16,
    pub height: u16,
    pub error: Option<String>,
}

/// 表单操作结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormAction {
    Continue,
    Submit,
    Cancel,
}

impl FormState {
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

    /// 按字段名取值
    pub fn value(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|f| f.name == name)
            .map(|f| f.value.as_str())
    }

    /// 文本字段数量（不含 device 列表等扩展选区）
    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    pub fn set_error(&mut self, msg: String) {
        self.error = Some(msg);
    }

    pub fn clear_error(&mut self) {
        self.error = None;
    }

    pub fn focused_field_name(&self) -> Option<&'static str> {
        self.fields.get(self.focus).map(|f| f.name)
    }

    /// 将 Device ID 输入格式化为 8 组、每组 7 个字母数字，用 `-` 连接。
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

    pub fn is_on_list(&self) -> bool {
        self.focus == self.fields.len()
    }
}
