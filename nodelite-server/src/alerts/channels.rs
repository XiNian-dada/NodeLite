//! 告警投递渠道集合。
//!
//! 投递结果按渠道记录：一个渠道失败时只重试它，已成功的渠道不会重复收到同一条消息。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChannelSet {
    pub(crate) webhook: bool,
    pub(crate) smtp: bool,
}

impl ChannelSet {
    pub(crate) const ALL: Self = Self {
        webhook: true,
        smtp: true,
    };
    pub(crate) const NONE: Self = Self {
        webhook: false,
        smtp: false,
    };

    pub(crate) fn is_empty(self) -> bool {
        !self.webhook && !self.smtp
    }

    pub(crate) fn union(self, other: Self) -> Self {
        Self {
            webhook: self.webhook || other.webhook,
            smtp: self.smtp || other.smtp,
        }
    }

    pub(crate) fn without(self, other: Self) -> Self {
        Self {
            webhook: self.webhook && !other.webhook,
            smtp: self.smtp && !other.smtp,
        }
    }
}

impl Default for ChannelSet {
    fn default() -> Self {
        Self::ALL
    }
}

#[cfg(test)]
mod tests {
    use super::ChannelSet;

    #[test]
    fn set_operations_combine_per_channel() {
        let webhook = ChannelSet {
            webhook: true,
            smtp: false,
        };
        assert!(ChannelSet::NONE.is_empty());
        assert_eq!(ChannelSet::NONE.union(webhook), webhook);
        assert_eq!(
            ChannelSet::ALL.without(webhook),
            ChannelSet {
                webhook: false,
                smtp: true
            }
        );
        assert!(webhook.without(ChannelSet::ALL).is_empty());
    }
}
