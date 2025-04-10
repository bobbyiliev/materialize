// Copyright Materialize, Inc. and contributors. All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use std::fmt;

use chrono::{
    DateTime, Duration, FixedOffset, NaiveDateTime, NaiveTime, Offset, TimeZone, Timelike, Utc,
};
use mz_lowertest::MzReflect;
use mz_ore::result::ResultExt;
use mz_pgtz::timezone::Timezone;
use mz_repr::adt::date::Date;
use mz_repr::adt::datetime::DateTimeUnits;
use mz_repr::adt::interval::Interval;
use mz_repr::adt::numeric::{DecimalLike, Numeric};
use mz_repr::adt::timestamp::{CheckedTimestamp, MAX_PRECISION, TimestampPrecision};
use mz_repr::{ColumnType, ScalarType, strconv};
use proptest_derive::Arbitrary;
use serde::{Deserialize, Serialize};

use crate::EvalError;
use crate::scalar::func::format::DateTimeFormat;
use crate::scalar::func::{EagerUnaryFunc, TimestampLike};

sqlfunc!(
    #[sqlname = "timestamp_to_text"]
    #[preserves_uniqueness = true]
    #[inverse = to_unary!(super::CastStringToTimestamp(None))]
    fn cast_timestamp_to_string(a: CheckedTimestamp<NaiveDateTime>) -> String {
        let mut buf = String::new();
        strconv::format_timestamp(&mut buf, &a);
        buf
    }
);

sqlfunc!(
    #[sqlname = "timestamp_with_time_zone_to_text"]
    #[preserves_uniqueness = true]
    #[inverse = to_unary!(super::CastStringToTimestampTz(None))]
    fn cast_timestamp_tz_to_string(a: CheckedTimestamp<DateTime<Utc>>) -> String {
        let mut buf = String::new();
        strconv::format_timestamptz(&mut buf, &a);
        buf
    }
);

sqlfunc!(
    #[sqlname = "timestamp_to_date"]
    #[preserves_uniqueness = false]
    #[inverse = to_unary!(super::CastDateToTimestamp(None))]
    #[is_monotone = true]
    fn cast_timestamp_to_date(a: CheckedTimestamp<NaiveDateTime>) -> Result<Date, EvalError> {
        Ok(a.date().try_into()?)
    }
);

sqlfunc!(
    #[sqlname = "timestamp_with_time_zone_to_date"]
    #[preserves_uniqueness = false]
    #[inverse = to_unary!(super::CastDateToTimestampTz(None))]
    #[is_monotone = true]
    fn cast_timestamp_tz_to_date(a: CheckedTimestamp<DateTime<Utc>>) -> Result<Date, EvalError> {
        Ok(a.naive_utc().date().try_into()?)
    }
);

#[derive(
    Arbitrary, Ord, PartialOrd, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Hash, MzReflect,
)]
pub struct CastTimestampToTimestampTz {
    pub from: Option<TimestampPrecision>,
    pub to: Option<TimestampPrecision>,
}

impl<'a> EagerUnaryFunc<'a> for CastTimestampToTimestampTz {
    type Input = CheckedTimestamp<NaiveDateTime>;
    type Output = Result<CheckedTimestamp<DateTime<Utc>>, EvalError>;

    fn call(
        &self,
        a: CheckedTimestamp<NaiveDateTime>,
    ) -> Result<CheckedTimestamp<DateTime<Utc>>, EvalError> {
        let out =
            CheckedTimestamp::try_from(DateTime::<Utc>::from_naive_utc_and_offset(a.into(), Utc))?;
        let updated = out.round_to_precision(self.to)?;
        Ok(updated)
    }

    fn output_type(&self, input: ColumnType) -> ColumnType {
        ScalarType::TimestampTz { precision: self.to }.nullable(input.nullable)
    }

    fn preserves_uniqueness(&self) -> bool {
        let to_p = self.to.map(|p| p.into_u8()).unwrap_or(MAX_PRECISION);
        let from_p = self.from.map(|p| p.into_u8()).unwrap_or(MAX_PRECISION);
        // If it's getting cast to a higher precision, it should preserve uniqueness but not otherwise.
        to_p >= from_p
    }

    fn inverse(&self) -> Option<crate::UnaryFunc> {
        to_unary!(super::CastTimestampTzToTimestamp {
            from: self.from,
            to: self.to
        })
    }

    fn is_monotone(&self) -> bool {
        true
    }
}

impl fmt::Display for CastTimestampToTimestampTz {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("timestamp_to_timestamp_with_time_zone")
    }
}

#[derive(
    Arbitrary, Ord, PartialOrd, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Hash, MzReflect,
)]
pub struct AdjustTimestampPrecision {
    pub from: Option<TimestampPrecision>,
    pub to: Option<TimestampPrecision>,
}

impl<'a> EagerUnaryFunc<'a> for AdjustTimestampPrecision {
    type Input = CheckedTimestamp<NaiveDateTime>;
    type Output = Result<CheckedTimestamp<NaiveDateTime>, EvalError>;

    fn call(
        &self,
        a: CheckedTimestamp<NaiveDateTime>,
    ) -> Result<CheckedTimestamp<NaiveDateTime>, EvalError> {
        // This should never have been called if precisions are same.
        // Adding a soft-assert to flag if there are such instances.
        mz_ore::soft_assert_no_log!(self.to != self.from);

        let updated = a.round_to_precision(self.to)?;
        Ok(updated)
    }

    fn output_type(&self, input: ColumnType) -> ColumnType {
        ScalarType::Timestamp { precision: self.to }.nullable(input.nullable)
    }

    fn preserves_uniqueness(&self) -> bool {
        let to_p = self.to.map(|p| p.into_u8()).unwrap_or(MAX_PRECISION);
        let from_p = self.from.map(|p| p.into_u8()).unwrap_or(MAX_PRECISION);
        // If it's getting cast to a higher precision, it should preserve uniqueness but not otherwise.
        to_p >= from_p
    }

    fn inverse(&self) -> Option<crate::UnaryFunc> {
        None
    }

    fn is_monotone(&self) -> bool {
        true
    }
}

impl fmt::Display for AdjustTimestampPrecision {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("adjust_timestamp_precision")
    }
}

#[derive(
    Arbitrary, Ord, PartialOrd, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Hash, MzReflect,
)]
pub struct CastTimestampTzToTimestamp {
    pub from: Option<TimestampPrecision>,
    pub to: Option<TimestampPrecision>,
}

impl<'a> EagerUnaryFunc<'a> for CastTimestampTzToTimestamp {
    type Input = CheckedTimestamp<DateTime<Utc>>;
    type Output = Result<CheckedTimestamp<NaiveDateTime>, EvalError>;

    fn call(
        &self,
        a: CheckedTimestamp<DateTime<Utc>>,
    ) -> Result<CheckedTimestamp<NaiveDateTime>, EvalError> {
        let out = CheckedTimestamp::try_from(a.naive_utc())?;
        let updated = out.round_to_precision(self.to)?;
        Ok(updated)
    }

    fn output_type(&self, input: ColumnType) -> ColumnType {
        ScalarType::Timestamp { precision: self.to }.nullable(input.nullable)
    }

    fn preserves_uniqueness(&self) -> bool {
        let to_p = self.to.map(|p| p.into_u8()).unwrap_or(MAX_PRECISION);
        let from_p = self.from.map(|p| p.into_u8()).unwrap_or(MAX_PRECISION);
        // If it's getting cast to a higher precision, it should preserve uniqueness but not otherwise.
        to_p >= from_p
    }

    fn inverse(&self) -> Option<crate::UnaryFunc> {
        to_unary!(super::CastTimestampToTimestampTz {
            from: self.from,
            to: self.to
        })
    }

    fn is_monotone(&self) -> bool {
        true
    }
}

impl fmt::Display for CastTimestampTzToTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("timestamp_with_time_zone_to_timestamp")
    }
}

#[derive(
    Arbitrary, Ord, PartialOrd, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Hash, MzReflect,
)]
pub struct AdjustTimestampTzPrecision {
    pub from: Option<TimestampPrecision>,
    pub to: Option<TimestampPrecision>,
}

impl<'a> EagerUnaryFunc<'a> for AdjustTimestampTzPrecision {
    type Input = CheckedTimestamp<DateTime<Utc>>;
    type Output = Result<CheckedTimestamp<DateTime<Utc>>, EvalError>;

    fn call(
        &self,
        a: CheckedTimestamp<DateTime<Utc>>,
    ) -> Result<CheckedTimestamp<DateTime<Utc>>, EvalError> {
        // This should never have been called if precisions are same.
        // Adding a soft-assert to flag if there are such instances.
        mz_ore::soft_assert_no_log!(self.to != self.from);

        let updated = a.round_to_precision(self.to)?;
        Ok(updated)
    }

    fn output_type(&self, input: ColumnType) -> ColumnType {
        ScalarType::TimestampTz { precision: self.to }.nullable(input.nullable)
    }

    fn preserves_uniqueness(&self) -> bool {
        let to_p = self.to.map(|p| p.into_u8()).unwrap_or(MAX_PRECISION);
        let from_p = self.from.map(|p| p.into_u8()).unwrap_or(MAX_PRECISION);
        // If it's getting cast to a higher precision, it should preserve uniqueness but not otherwise.
        to_p >= from_p
    }

    fn inverse(&self) -> Option<crate::UnaryFunc> {
        None
    }

    fn is_monotone(&self) -> bool {
        true
    }
}

impl fmt::Display for AdjustTimestampTzPrecision {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("adjust_timestamp_with_time_zone_precision")
    }
}

sqlfunc!(
    #[sqlname = "timestamp_to_time"]
    #[preserves_uniqueness = false]
    fn cast_timestamp_to_time(a: CheckedTimestamp<NaiveDateTime>) -> NaiveTime {
        a.time()
    }
);

sqlfunc!(
    #[sqlname = "timestamp_with_time_zone_to_time"]
    #[preserves_uniqueness = false]
    fn cast_timestamp_tz_to_time(a: CheckedTimestamp<DateTime<Utc>>) -> NaiveTime {
        a.naive_utc().time()
    }
);

pub fn date_part_interval_inner<D>(units: DateTimeUnits, interval: Interval) -> Result<D, EvalError>
where
    D: DecimalLike,
{
    match units {
        DateTimeUnits::Epoch => Ok(interval.as_epoch_seconds()),
        DateTimeUnits::Millennium => Ok(D::from(interval.millennia())),
        DateTimeUnits::Century => Ok(D::from(interval.centuries())),
        DateTimeUnits::Decade => Ok(D::from(interval.decades())),
        DateTimeUnits::Year => Ok(D::from(interval.years())),
        DateTimeUnits::Quarter => Ok(D::from(interval.quarters())),
        DateTimeUnits::Month => Ok(D::from(interval.months())),
        DateTimeUnits::Day => Ok(D::lossy_from(interval.days())),
        DateTimeUnits::Hour => Ok(D::lossy_from(interval.hours())),
        DateTimeUnits::Minute => Ok(D::lossy_from(interval.minutes())),
        DateTimeUnits::Second => Ok(interval.seconds()),
        DateTimeUnits::Milliseconds => Ok(interval.milliseconds()),
        DateTimeUnits::Microseconds => Ok(interval.microseconds()),
        DateTimeUnits::Week
        | DateTimeUnits::Timezone
        | DateTimeUnits::TimezoneHour
        | DateTimeUnits::TimezoneMinute
        | DateTimeUnits::DayOfWeek
        | DateTimeUnits::DayOfYear
        | DateTimeUnits::IsoDayOfWeek
        | DateTimeUnits::IsoDayOfYear => Err(EvalError::Unsupported {
            feature: format!("'{}' timestamp units", units).into(),
            discussion_no: None,
        }),
    }
}

#[derive(
    Arbitrary, Ord, PartialOrd, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Hash, MzReflect,
)]
pub struct ExtractInterval(pub DateTimeUnits);

impl<'a> EagerUnaryFunc<'a> for ExtractInterval {
    type Input = Interval;
    type Output = Result<Numeric, EvalError>;

    fn call(&self, a: Interval) -> Result<Numeric, EvalError> {
        date_part_interval_inner(self.0, a)
    }

    fn output_type(&self, input: ColumnType) -> ColumnType {
        ScalarType::Numeric { max_scale: None }.nullable(input.nullable)
    }
}

impl fmt::Display for ExtractInterval {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "extract_{}_iv", self.0)
    }
}

#[derive(
    Arbitrary, Ord, PartialOrd, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Hash, MzReflect,
)]
pub struct DatePartInterval(pub DateTimeUnits);

impl<'a> EagerUnaryFunc<'a> for DatePartInterval {
    type Input = Interval;
    type Output = Result<f64, EvalError>;

    fn call(&self, a: Interval) -> Result<f64, EvalError> {
        date_part_interval_inner(self.0, a)
    }

    fn output_type(&self, input: ColumnType) -> ColumnType {
        ScalarType::Float64.nullable(input.nullable)
    }
}

impl fmt::Display for DatePartInterval {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "date_part_{}_iv", self.0)
    }
}

pub fn date_part_timestamp_inner<T, D>(units: DateTimeUnits, ts: &T) -> Result<D, EvalError>
where
    T: TimestampLike,
    D: DecimalLike,
{
    match units {
        DateTimeUnits::Epoch => Ok(TimestampLike::extract_epoch(ts)),
        DateTimeUnits::Millennium => Ok(D::from(ts.millennium())),
        DateTimeUnits::Century => Ok(D::from(ts.century())),
        DateTimeUnits::Decade => Ok(D::from(ts.decade())),
        DateTimeUnits::Year => Ok(D::from(ts.year())),
        DateTimeUnits::Quarter => Ok(D::from(ts.quarter())),
        DateTimeUnits::Week => Ok(D::from(ts.iso_week_number())),
        DateTimeUnits::Month => Ok(D::from(ts.month())),
        DateTimeUnits::Day => Ok(D::from(ts.day())),
        DateTimeUnits::DayOfWeek => Ok(D::from(ts.day_of_week())),
        DateTimeUnits::DayOfYear => Ok(D::from(ts.ordinal())),
        DateTimeUnits::IsoDayOfWeek => Ok(D::from(ts.iso_day_of_week())),
        DateTimeUnits::Hour => Ok(D::from(ts.hour())),
        DateTimeUnits::Minute => Ok(D::from(ts.minute())),
        DateTimeUnits::Second => Ok(ts.extract_second()),
        DateTimeUnits::Milliseconds => Ok(ts.extract_millisecond()),
        DateTimeUnits::Microseconds => Ok(ts.extract_microsecond()),
        DateTimeUnits::Timezone
        | DateTimeUnits::TimezoneHour
        | DateTimeUnits::TimezoneMinute
        | DateTimeUnits::IsoDayOfYear => Err(EvalError::Unsupported {
            feature: format!("'{}' timestamp units", units).into(),
            discussion_no: None,
        }),
    }
}

/// Will extracting this unit from the timestamp include the "most significant bits" of
/// the timestamp?
pub(crate) fn most_significant_unit(unit: DateTimeUnits) -> bool {
    match unit {
        DateTimeUnits::Epoch
        | DateTimeUnits::Millennium
        | DateTimeUnits::Century
        | DateTimeUnits::Decade
        | DateTimeUnits::Year => true,
        _ => false,
    }
}

#[derive(
    Arbitrary, Ord, PartialOrd, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Hash, MzReflect,
)]
pub struct ExtractTimestamp(pub DateTimeUnits);

impl<'a> EagerUnaryFunc<'a> for ExtractTimestamp {
    type Input = CheckedTimestamp<NaiveDateTime>;
    type Output = Result<Numeric, EvalError>;

    fn call(&self, a: CheckedTimestamp<NaiveDateTime>) -> Result<Numeric, EvalError> {
        date_part_timestamp_inner(self.0, &*a)
    }

    fn output_type(&self, input: ColumnType) -> ColumnType {
        ScalarType::Numeric { max_scale: None }.nullable(input.nullable)
    }

    fn is_monotone(&self) -> bool {
        most_significant_unit(self.0)
    }
}

impl fmt::Display for ExtractTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "extract_{}_ts", self.0)
    }
}

#[derive(
    Arbitrary, Ord, PartialOrd, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Hash, MzReflect,
)]
pub struct ExtractTimestampTz(pub DateTimeUnits);

impl<'a> EagerUnaryFunc<'a> for ExtractTimestampTz {
    type Input = CheckedTimestamp<DateTime<Utc>>;
    type Output = Result<Numeric, EvalError>;

    fn call(&self, a: CheckedTimestamp<DateTime<Utc>>) -> Result<Numeric, EvalError> {
        date_part_timestamp_inner(self.0, &*a)
    }

    fn output_type(&self, input: ColumnType) -> ColumnType {
        ScalarType::Numeric { max_scale: None }.nullable(input.nullable)
    }

    fn is_monotone(&self) -> bool {
        // Unlike the timezone-less timestamp, it's not safe to extract the "high-order bits" like
        // year: year takes timezone into account, and it's quite possible for a different timezone
        // to be in a previous year while having a later UTC-equivalent time.
        self.0 == DateTimeUnits::Epoch
    }
}

impl fmt::Display for ExtractTimestampTz {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "extract_{}_tstz", self.0)
    }
}

#[derive(
    Arbitrary, Ord, PartialOrd, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Hash, MzReflect,
)]
pub struct DatePartTimestamp(pub DateTimeUnits);

impl<'a> EagerUnaryFunc<'a> for DatePartTimestamp {
    type Input = CheckedTimestamp<NaiveDateTime>;
    type Output = Result<f64, EvalError>;

    fn call(&self, a: CheckedTimestamp<NaiveDateTime>) -> Result<f64, EvalError> {
        date_part_timestamp_inner(self.0, &*a)
    }

    fn output_type(&self, input: ColumnType) -> ColumnType {
        ScalarType::Float64.nullable(input.nullable)
    }
}

impl fmt::Display for DatePartTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "date_part_{}_ts", self.0)
    }
}

#[derive(
    Arbitrary, Ord, PartialOrd, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Hash, MzReflect,
)]
pub struct DatePartTimestampTz(pub DateTimeUnits);

impl<'a> EagerUnaryFunc<'a> for DatePartTimestampTz {
    type Input = CheckedTimestamp<DateTime<Utc>>;
    type Output = Result<f64, EvalError>;

    fn call(&self, a: CheckedTimestamp<DateTime<Utc>>) -> Result<f64, EvalError> {
        date_part_timestamp_inner(self.0, &*a)
    }

    fn output_type(&self, input: ColumnType) -> ColumnType {
        ScalarType::Float64.nullable(input.nullable)
    }
}

impl fmt::Display for DatePartTimestampTz {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "date_part_{}_tstz", self.0)
    }
}

pub fn date_trunc_inner<T: TimestampLike>(units: DateTimeUnits, ts: &T) -> Result<T, EvalError> {
    match units {
        DateTimeUnits::Millennium => Ok(ts.truncate_millennium()),
        DateTimeUnits::Century => Ok(ts.truncate_century()),
        DateTimeUnits::Decade => Ok(ts.truncate_decade()),
        DateTimeUnits::Year => Ok(ts.truncate_year()),
        DateTimeUnits::Quarter => Ok(ts.truncate_quarter()),
        DateTimeUnits::Week => Ok(ts.truncate_week()?),
        DateTimeUnits::Day => Ok(ts.truncate_day()),
        DateTimeUnits::Hour => Ok(ts.truncate_hour()),
        DateTimeUnits::Minute => Ok(ts.truncate_minute()),
        DateTimeUnits::Second => Ok(ts.truncate_second()),
        DateTimeUnits::Month => Ok(ts.truncate_month()),
        DateTimeUnits::Milliseconds => Ok(ts.truncate_milliseconds()),
        DateTimeUnits::Microseconds => Ok(ts.truncate_microseconds()),
        DateTimeUnits::Epoch
        | DateTimeUnits::Timezone
        | DateTimeUnits::TimezoneHour
        | DateTimeUnits::TimezoneMinute
        | DateTimeUnits::DayOfWeek
        | DateTimeUnits::DayOfYear
        | DateTimeUnits::IsoDayOfWeek
        | DateTimeUnits::IsoDayOfYear => Err(EvalError::Unsupported {
            feature: format!("'{}' timestamp units", units).into(),
            discussion_no: None,
        }),
    }
}

#[derive(
    Arbitrary, Ord, PartialOrd, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Hash, MzReflect,
)]
pub struct DateTruncTimestamp(pub DateTimeUnits);

impl<'a> EagerUnaryFunc<'a> for DateTruncTimestamp {
    type Input = CheckedTimestamp<NaiveDateTime>;
    type Output = Result<CheckedTimestamp<NaiveDateTime>, EvalError>;

    fn call(
        &self,
        a: CheckedTimestamp<NaiveDateTime>,
    ) -> Result<CheckedTimestamp<NaiveDateTime>, EvalError> {
        date_trunc_inner(self.0, &*a)?.try_into().err_into()
    }

    fn output_type(&self, input: ColumnType) -> ColumnType {
        ScalarType::Timestamp { precision: None }.nullable(input.nullable)
    }

    fn is_monotone(&self) -> bool {
        true
    }
}

impl fmt::Display for DateTruncTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "date_trunc_{}_ts", self.0)
    }
}

#[derive(
    Arbitrary, Ord, PartialOrd, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Hash, MzReflect,
)]
pub struct DateTruncTimestampTz(pub DateTimeUnits);

impl<'a> EagerUnaryFunc<'a> for DateTruncTimestampTz {
    type Input = CheckedTimestamp<DateTime<Utc>>;
    type Output = Result<CheckedTimestamp<DateTime<Utc>>, EvalError>;

    fn call(
        &self,
        a: CheckedTimestamp<DateTime<Utc>>,
    ) -> Result<CheckedTimestamp<DateTime<Utc>>, EvalError> {
        date_trunc_inner(self.0, &*a)?.try_into().err_into()
    }

    fn output_type(&self, input: ColumnType) -> ColumnType {
        ScalarType::TimestampTz { precision: None }.nullable(input.nullable)
    }

    fn is_monotone(&self) -> bool {
        true
    }
}

impl fmt::Display for DateTruncTimestampTz {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "date_trunc_{}_tstz", self.0)
    }
}

/// Converts the timestamp `dt`, which is assumed to be in the time of the timezone `tz` to a timestamptz in UTC.
/// This operation is fallible because certain timestamps at timezones that observe DST are simply impossible or
/// ambiguous. In case of ambiguity (when a hour repeats) we will prefer the latest variant, and when an hour is
/// impossible, we will attempt to fix it by advancing it. For example, `EST` and `2020-11-11T12:39:14` would return
/// `2020-11-11T17:39:14Z`. A DST observing timezone like `America/New_York` would cause the following DST anomalies:
/// `2020-11-01T00:59:59` -> `2020-11-01T04:59:59Z` and `2020-11-01T01:00:00` -> `2020-11-01T06:00:00Z`
/// `2020-03-08T02:59:59` -> `2020-03-08T07:59:59Z` and `2020-03-08T03:00:00` -> `2020-03-08T07:00:00Z`
pub fn timezone_timestamp(
    tz: Timezone,
    dt: NaiveDateTime,
) -> Result<CheckedTimestamp<DateTime<Utc>>, EvalError> {
    let offset = match tz {
        Timezone::FixedOffset(offset) => offset,
        Timezone::Tz(tz) => match tz.offset_from_local_datetime(&dt).latest() {
            Some(offset) => offset.fix(),
            None => {
                let dt = dt
                    .checked_add_signed(
                        Duration::try_hours(1).ok_or(EvalError::TimestampOutOfRange)?,
                    )
                    .ok_or(EvalError::TimestampOutOfRange)?;
                tz.offset_from_local_datetime(&dt)
                    .latest()
                    .ok_or(EvalError::InvalidTimezoneConversion)?
                    .fix()
            }
        },
    };
    DateTime::from_naive_utc_and_offset(dt - offset, Utc)
        .try_into()
        .err_into()
}

/// Converts the UTC timestamptz `utc` to the local timestamp of the timezone `tz`.
/// For example, `EST` and `2020-11-11T17:39:14Z` would return `2020-11-11T12:39:14`.
pub fn timezone_timestamptz(tz: Timezone, utc: DateTime<Utc>) -> Result<NaiveDateTime, EvalError> {
    let offset = match tz {
        Timezone::FixedOffset(offset) => offset,
        Timezone::Tz(tz) => tz.offset_from_utc_datetime(&utc.naive_utc()).fix(),
    };
    checked_add_with_leapsecond(&utc.naive_utc(), &offset).ok_or(EvalError::TimestampOutOfRange)
}

/// Checked addition that is missing from chrono. Adapt its methods here but add a check.
fn checked_add_with_leapsecond(lhs: &NaiveDateTime, rhs: &FixedOffset) -> Option<NaiveDateTime> {
    // extract and temporarily remove the fractional part and later recover it
    let nanos = lhs.nanosecond();
    let lhs = lhs.with_nanosecond(0).unwrap();
    let rhs = rhs.local_minus_utc();
    lhs.checked_add_signed(match chrono::Duration::try_seconds(i64::from(rhs)) {
        Some(dur) => dur,
        None => return None,
    })
    .map(|dt| dt.with_nanosecond(nanos).unwrap())
}

#[derive(
    Arbitrary, Ord, PartialOrd, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Hash, MzReflect,
)]
pub struct TimezoneTimestamp(pub Timezone);

impl<'a> EagerUnaryFunc<'a> for TimezoneTimestamp {
    type Input = CheckedTimestamp<NaiveDateTime>;
    type Output = Result<CheckedTimestamp<DateTime<Utc>>, EvalError>;

    fn call(
        &self,
        a: CheckedTimestamp<NaiveDateTime>,
    ) -> Result<CheckedTimestamp<DateTime<Utc>>, EvalError> {
        timezone_timestamp(self.0, a.to_naive())
    }

    fn output_type(&self, input: ColumnType) -> ColumnType {
        ScalarType::TimestampTz { precision: None }.nullable(input.nullable)
    }
}

impl fmt::Display for TimezoneTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "timezone_{}_ts", self.0)
    }
}

#[derive(
    Arbitrary, Ord, PartialOrd, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Hash, MzReflect,
)]
pub struct TimezoneTimestampTz(pub Timezone);

impl<'a> EagerUnaryFunc<'a> for TimezoneTimestampTz {
    type Input = CheckedTimestamp<DateTime<Utc>>;
    type Output = Result<CheckedTimestamp<NaiveDateTime>, EvalError>;

    fn call(
        &self,
        a: CheckedTimestamp<DateTime<Utc>>,
    ) -> Result<CheckedTimestamp<NaiveDateTime>, EvalError> {
        timezone_timestamptz(self.0, a.into())?
            .try_into()
            .err_into()
    }

    fn output_type(&self, input: ColumnType) -> ColumnType {
        ScalarType::Timestamp { precision: None }.nullable(input.nullable)
    }
}

impl fmt::Display for TimezoneTimestampTz {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "timezone_{}_tstz", self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, MzReflect)]
pub struct ToCharTimestamp {
    pub format_string: String,
    pub format: DateTimeFormat,
}

impl<'a> EagerUnaryFunc<'a> for ToCharTimestamp {
    type Input = CheckedTimestamp<NaiveDateTime>;
    type Output = String;

    fn call(&self, input: CheckedTimestamp<NaiveDateTime>) -> String {
        self.format.render(&*input)
    }

    fn output_type(&self, input: ColumnType) -> ColumnType {
        ScalarType::String.nullable(input.nullable)
    }
}

impl fmt::Display for ToCharTimestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tocharts[{}]", self.format_string)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, MzReflect)]
pub struct ToCharTimestampTz {
    pub format_string: String,
    pub format: DateTimeFormat,
}

impl<'a> EagerUnaryFunc<'a> for ToCharTimestampTz {
    type Input = CheckedTimestamp<DateTime<Utc>>;
    type Output = String;

    fn call(&self, input: CheckedTimestamp<DateTime<Utc>>) -> String {
        self.format.render(&*input)
    }

    fn output_type(&self, input: ColumnType) -> ColumnType {
        ScalarType::String.nullable(input.nullable)
    }
}

impl fmt::Display for ToCharTimestampTz {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tochartstz[{}]", self.format_string)
    }
}
