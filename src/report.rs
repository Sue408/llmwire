//! 转换质量报告。
//!
//! 报告显式记录字段降级、不可表达内容与策略阻断，避免静默丢失。
use std::fmt;

use crate::ids::OpaqueKind;

/// 报告条目的严重程度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// 可安全忽略的信息。
    Silent,
    /// 已采用有损降级。
    Degraded,
    /// 当前策略下无法安全继续。
    Fatal,
}

/// 字段无法映射到目标协议的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnmappedReason {
    /// 目标协议不支持该字段。
    UnsupportedByTarget,
    /// IR 或目标协议无法表达该语义。
    NotRepresentable,
    /// Host 策略阻止了转换。
    PolicyBlocked,
}

/// 一个无法映射的字段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unmapped {
    /// 字段路径或稳定名称。
    pub field: Box<str>,
    /// 无法映射的原因。
    pub reason: UnmappedReason,
    /// 严重程度。
    pub severity: Severity,
}

/// 一条降级或诊断警告。
///
/// `Debug` 只显示消息长度，避免记录敏感内容。
#[derive(Clone, PartialEq, Eq)]
pub struct Warning {
    /// 字段路径或稳定名称。
    pub field: Box<str>,
    /// 面向诊断的消息。
    pub message: Box<str>,
    /// 严重程度。
    pub severity: Severity,
}

impl fmt::Debug for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Warning")
            .field("field", &self.field)
            .field("message_len", &self.message.len())
            .field("severity", &self.severity)
            .finish()
    }
}

/// 一次请求转换累积的质量报告。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// 无法映射的字段。
    pub unmapped: Vec<Unmapped>,
    /// 降级与诊断告警。
    pub warnings: Vec<Warning>,
}

impl Report {
    /// 创建空报告。
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录一个无法映射的字段。
    pub fn unmapped(
        &mut self,
        field: impl Into<Box<str>>,
        reason: UnmappedReason,
        severity: Severity,
    ) {
        self.unmapped.push(Unmapped {
            field: field.into(),
            reason,
            severity,
        });
    }

    /// 记录一条降级或诊断告警。
    pub fn warn(
        &mut self,
        field: impl Into<Box<str>>,
        message: impl Into<Box<str>>,
        severity: Severity,
    ) {
        self.warnings.push(Warning {
            field: field.into(),
            message: message.into(),
            severity,
        });
    }

    /// 记录不透明内容，仅保留 kind 与长度。
    pub fn opaque(
        &mut self,
        field: impl Into<Box<str>>,
        kind: OpaqueKind,
        len: usize,
        severity: Severity,
    ) {
        self.warn(field, format!("opaque kind={kind:?} len={len}"), severity);
    }

    /// 合并另一份报告。
    pub fn merge(&mut self, other: Report) {
        self.unmapped.extend(other.unmapped);
        self.warnings.extend(other.warnings);
    }

    /// 判断报告是否没有任何条目。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.unmapped.is_empty() && self.warnings.is_empty()
    }

    /// 判断是否包含 `Fatal` 条目。
    #[must_use]
    pub fn has_fatal(&self) -> bool {
        self.unmapped
            .iter()
            .any(|entry| entry.severity == Severity::Fatal)
            || self
                .warnings
                .iter()
                .any(|entry| entry.severity == Severity::Fatal)
    }
}
