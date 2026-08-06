//! Portable six-field cron parsing used by native schedule generators.

use std::collections::BTreeSet;

use anyhow::{Context, Result, bail};

/// 已展开为离散取值集合的六段式 cron 表达式。
#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "schedule"), allow(dead_code))]
pub(crate) struct CronExpression {
    second: CronField,
    minute: CronField,
    hour: CronField,
    day_of_month: CronField,
    month: CronField,
    day_of_week: CronField,
}

/// 单个 cron 字段的有序值集合及“未限制”标志。
#[derive(Debug, Clone)]
#[cfg_attr(not(feature = "schedule"), allow(dead_code))]
pub(crate) struct CronField {
    values: Vec<u8>,
    unrestricted: bool,
}

/// 字段类别决定有效范围、名称别名和星期归一化规则。
#[derive(Debug, Clone, Copy)]
enum FieldKind {
    Second,
    Minute,
    Hour,
    DayOfMonth,
    Month,
    DayOfWeek,
}

#[cfg_attr(not(feature = "schedule"), allow(dead_code))]
impl CronExpression {
    /// 解析六段式表达式，并拒绝原生调度器无法一致表达的日期语义。
    pub(crate) fn parse(expression: &str) -> Result<Self> {
        let parts = expression.split_whitespace().collect::<Vec<_>>();
        if parts.len() != 6 {
            bail!(
                "cron expression must contain exactly 6 fields: second minute hour day month weekday"
            );
        }
        let parsed = Self {
            second: CronField::parse(parts[0], FieldKind::Second)
                .context("invalid cron second field")?,
            minute: CronField::parse(parts[1], FieldKind::Minute)
                .context("invalid cron minute field")?,
            hour: CronField::parse(parts[2], FieldKind::Hour).context("invalid cron hour field")?,
            day_of_month: CronField::parse(parts[3], FieldKind::DayOfMonth)
                .context("invalid cron day-of-month field")?,
            month: CronField::parse(parts[4], FieldKind::Month)
                .context("invalid cron month field")?,
            day_of_week: CronField::parse(parts[5], FieldKind::DayOfWeek)
                .context("invalid cron day-of-week field")?,
        };
        // Unix cron 对“日”和“周”使用 OR，但部分原生平台使用 AND，故禁止同时限制。
        if !parsed.day_of_month.unrestricted && !parsed.day_of_week.unrestricted {
            bail!(
                "cron cannot restrict both day-of-month and day-of-week because native schedulers do not preserve Unix cron OR semantics"
            );
        }
        Ok(parsed)
    }

    pub(crate) fn seconds(&self) -> &[u8] {
        &self.second.values
    }

    pub(crate) fn minutes(&self) -> &[u8] {
        &self.minute.values
    }

    pub(crate) fn hours(&self) -> &[u8] {
        &self.hour.values
    }

    pub(crate) fn days_of_month(&self) -> &[u8] {
        &self.day_of_month.values
    }

    pub(crate) fn months(&self) -> &[u8] {
        &self.month.values
    }

    pub(crate) fn days_of_week(&self) -> &[u8] {
        &self.day_of_week.values
    }

    pub(crate) fn seconds_unrestricted(&self) -> bool {
        self.second.unrestricted
    }

    pub(crate) fn minutes_unrestricted(&self) -> bool {
        self.minute.unrestricted
    }

    pub(crate) fn hours_unrestricted(&self) -> bool {
        self.hour.unrestricted
    }

    pub(crate) fn days_of_month_unrestricted(&self) -> bool {
        self.day_of_month.unrestricted
    }

    pub(crate) fn months_unrestricted(&self) -> bool {
        self.month.unrestricted
    }

    pub(crate) fn days_of_week_unrestricted(&self) -> bool {
        self.day_of_week.unrestricted
    }
}

impl CronField {
    /// 展开列表、范围、步长、通配符和名称别名为稳定有序的数值集合。
    fn parse(expression: &str, kind: FieldKind) -> Result<Self> {
        if expression.is_empty() {
            bail!("field cannot be empty");
        }
        let (minimum, maximum) = kind.bounds();
        // BTreeSet 同时完成去重和排序，方便后续生成平台配置。
        let mut values = BTreeSet::new();
        for item in expression.split(',') {
            if item.is_empty() {
                bail!("empty list item");
            }
            let (base, step) = match item.split_once('/') {
                Some((base, step)) => {
                    if base.is_empty() || step.is_empty() || step.contains('/') {
                        bail!("invalid step expression: {item}");
                    }
                    let step = step.parse::<u8>().context("step is not a number")?;
                    if step == 0 {
                        bail!("step must be positive");
                    }
                    (base, Some(step))
                }
                None => (item, None),
            };
            let (start, end) = if base == "*" || base == "?" {
                if base == "?" && !matches!(kind, FieldKind::DayOfMonth | FieldKind::DayOfWeek) {
                    bail!("'?' is only allowed in day-of-month or day-of-week");
                }
                if base == "?" && step.is_some() {
                    bail!("'?' cannot be combined with a step");
                }
                (minimum, maximum)
            } else if let Some((start, end)) = base.split_once('-') {
                let start = kind.parse_value(start)?;
                let end = kind.parse_value(end)?;
                if start > end {
                    bail!("range start exceeds range end: {base}");
                }
                (start, end)
            } else {
                if step.is_some() {
                    bail!("steps require '*' or a range: {item}");
                }
                let value = kind.parse_value(base)?;
                (value, value)
            };
            let step = step.unwrap_or(1);
            let mut value = start;
            while value <= end {
                values.insert(kind.normalize(value));
                let Some(next) = value.checked_add(step) else {
                    break;
                };
                value = next;
            }
        }
        if values.is_empty() {
            bail!("field selects no values");
        }
        let unrestricted = values.len() == kind.value_count();
        Ok(Self {
            values: values.into_iter().collect(),
            unrestricted,
        })
    }
}

impl FieldKind {
    /// 返回当前字段允许的闭区间。
    fn bounds(self) -> (u8, u8) {
        match self {
            Self::Second | Self::Minute => (0, 59),
            Self::Hour => (0, 23),
            Self::DayOfMonth => (1, 31),
            Self::Month => (1, 12),
            Self::DayOfWeek => (0, 7),
        }
    }

    /// 返回字段覆盖全范围时应包含的不同取值数。
    fn value_count(self) -> usize {
        match self {
            Self::Second | Self::Minute => 60,
            Self::Hour => 24,
            Self::DayOfMonth => 31,
            Self::Month => 12,
            Self::DayOfWeek => 7,
        }
    }

    /// 解析数字或月份、星期英文缩写，并检查范围。
    fn parse_value(self, raw: &str) -> Result<u8> {
        let upper = raw.to_ascii_uppercase();
        let named = match self {
            Self::Month => match upper.as_str() {
                "JAN" => Some(1),
                "FEB" => Some(2),
                "MAR" => Some(3),
                "APR" => Some(4),
                "MAY" => Some(5),
                "JUN" => Some(6),
                "JUL" => Some(7),
                "AUG" => Some(8),
                "SEP" => Some(9),
                "OCT" => Some(10),
                "NOV" => Some(11),
                "DEC" => Some(12),
                _ => None,
            },
            Self::DayOfWeek => match upper.as_str() {
                "SUN" => Some(0),
                "MON" => Some(1),
                "TUE" => Some(2),
                "WED" => Some(3),
                "THU" => Some(4),
                "FRI" => Some(5),
                "SAT" => Some(6),
                _ => None,
            },
            _ => None,
        };
        let value = match named {
            Some(value) => value,
            None => raw
                .parse::<u8>()
                .with_context(|| format!("invalid value: {raw}"))?,
        };
        let (minimum, maximum) = self.bounds();
        if value < minimum || value > maximum {
            bail!("value {raw} is outside {minimum}-{maximum}");
        }
        Ok(value)
    }

    /// 把星期日的别名 `7` 归一化为 `0`，便于去重和跨平台生成。
    fn normalize(self, value: u8) -> u8 {
        if matches!(self, Self::DayOfWeek) && value == 7 {
            0
        } else {
            value
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CronExpression;

    #[test]
    fn parses_wildcards_lists_ranges_steps_and_names() {
        let cron = CronExpression::parse("0 */15 9-17 * JAN,MAR MON-FRI").unwrap();
        assert_eq!(cron.seconds(), &[0]);
        assert_eq!(cron.minutes(), &[0, 15, 30, 45]);
        assert_eq!(cron.hours(), &[9, 10, 11, 12, 13, 14, 15, 16, 17]);
        assert_eq!(cron.months(), &[1, 3]);
        assert_eq!(cron.days_of_week(), &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn normalizes_sunday_and_rejects_nonportable_day_semantics() {
        let cron = CronExpression::parse("0 0 2 ? * 0,7").unwrap();
        assert_eq!(cron.days_of_week(), &[0]);
        assert!(CronExpression::parse("0 0 2 1 * MON").is_err());
    }

    #[test]
    fn rejects_invalid_field_counts_values_and_steps() {
        assert!(CronExpression::parse("0 2 * * *").is_err());
        assert!(CronExpression::parse("60 0 2 * * *").is_err());
        assert!(CronExpression::parse("*/0 0 2 * * *").is_err());
        assert!(CronExpression::parse("5/2 0 2 * * *").is_err());
    }
}
