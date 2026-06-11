//! 统一表单系统 — 替代 4 处重复的表单渲染和按键处理代码。
//!
//! 通过 `FormField` 的 `editable` 标志区分 add 模式（全部可编辑）与 edit 模式（ID 字段只读），
//! 消除 add_device/add_folder/edit_device/edit_folder 的 70-80% 重复。

pub mod handler;
pub mod render;

/// 表单字段定义
pub struct FormField {
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
        }
    }

    pub fn add_field(
        mut self,
        label: &'static str,
        value: String,
        editable: bool,
        hint: Option<&'static str>,
    ) -> Self {
        self.fields.push(FormField {
            label,
            value,
            editable,
            hint,
        });
        self
    }

    /// 文本字段数量（不含 device 列表等扩展选区）
    pub fn field_count(&self) -> usize {
        self.fields.len()
    }

    pub fn append_char(&mut self, c: char) {
        if let Some(field) = self.fields.get_mut(self.focus) {
            if field.editable {
                field.value.push(c);
            }
        }
    }

    pub fn backspace(&mut self) {
        if let Some(field) = self.fields.get_mut(self.focus) {
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
