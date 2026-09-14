use std::fmt;

use crate::ids::OpaqueKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Silent,
    Degraded,
    Fatal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnmappedReason {
    UnsupportedByTarget,
    NotRepresentable,
    PolicyBlocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unmapped {
    pub field: Box<str>,
    pub reason: UnmappedReason,
    pub severity: Severity,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Warning {
    pub field: Box<str>,
    pub message: Box<str>,
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub unmapped: Vec<Unmapped>,
    pub warnings: Vec<Warning>,
}

impl Report {
    pub fn new() -> Self {
        Self::default()
    }

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

    pub fn opaque(
        &mut self,
        field: impl Into<Box<str>>,
        kind: OpaqueKind,
        len: usize,
        severity: Severity,
    ) {
        self.warn(field, format!("opaque kind={kind:?} len={len}"), severity);
    }

    pub fn merge(&mut self, other: Report) {
        self.unmapped.extend(other.unmapped);
        self.warnings.extend(other.warnings);
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.unmapped.is_empty() && self.warnings.is_empty()
    }

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
