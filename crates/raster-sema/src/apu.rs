//! The `apu` register namespace (spec §9.1, §9.5): channel, then register.
//!
//! The data lives here and the judging with it, so `Analyzer::member_type` asks
//! one question — "is this a real APU register, and if not, say so once".

use raster_syntax::{Expression, Spanned};

use super::ValueType;

/// The channels with registers this release has, and those registers. The DMC
/// is deliberately absent: it is refused as not supported yet.
pub(crate) const CHANNELS: [(&str, &[&str]); 4] = [
    ("pulse1", &["volume", "sweep", "timer_lo", "timer_hi"]),
    ("pulse2", &["volume", "sweep", "timer_lo", "timer_hi"]),
    ("triangle", &["linear", "timer_lo", "timer_hi"]),
    ("noise", &["volume", "period", "length"]),
];

/// The registers that belong to no single channel, directly under `apu`.
pub(crate) const TOP_LEVEL: [&str; 2] = ["status", "frame_counter"];

/// The label when the channel is not one the APU has.
pub(crate) const CHANNEL_LIST: &str = "the APU has pulse1, pulse2, triangle and noise";

/// The label when a name directly under `apu` is neither a register nor a
/// channel register.
pub(crate) const APU_LIST: &str = "apu has the registers status and frame_counter, and the \
                                   channels pulse1, pulse2, triangle and noise";

pub(crate) const DMC_MESSAGE: &str = "the DMC registers are not supported yet";
pub(crate) const DMC_LABEL: &str =
    "how sample playback's cycle theft meets timed blocks is still open (§15.2)";

/// What a name directly under `apu` is.
pub(crate) enum Direct {
    /// `apu.status`, `apu.frame_counter`.
    Register,
    /// `apu.pulse1`: a channel used as a register.
    Channel,
    /// `apu.dmc`.
    Dmc,
    /// `apu.volume`: a channel's register, with the first channel that has it.
    BelongsTo(&'static str),
    /// Nothing of that name.
    Unknown,
}

pub(crate) fn direct(name: &str) -> Direct {
    if TOP_LEVEL.contains(&name) {
        Direct::Register
    } else if name == "dmc" {
        Direct::Dmc
    } else if CHANNELS.iter().any(|(channel, _)| *channel == name) {
        Direct::Channel
    } else if let Some((channel, _)) = CHANNELS
        .iter()
        .find(|(_, registers)| registers.contains(&name))
    {
        Direct::BelongsTo(channel)
    } else {
        Direct::Unknown
    }
}

/// The registers of `channel`, if it is one this release has.
pub(crate) fn registers(channel: &str) -> Option<&'static [&'static str]> {
    CHANNELS
        .iter()
        .find(|(name, _)| *name == channel)
        .map(|(_, registers)| *registers)
}

/// "volume, sweep, timer_lo and timer_hi".
pub(crate) fn english_list(names: &[&str]) -> String {
    match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        Some((last, _)) => (*last).to_owned(),
        None => String::new(),
    }
}

impl super::Analyzer {
    /// A name directly under `apu`: `apu.status`, `apu.pulse1`, `apu.volume`.
    /// One error with the carets under `whole`, or the register's type.
    pub(super) fn apu_direct_type(
        &mut self,
        whole: &Spanned<Expression>,
        member: &Spanned<String>,
    ) -> ValueType {
        match direct(&member.value) {
            Direct::Register => return ValueType::U8,
            Direct::Channel => self.error_with(
                whole.span,
                format!("`apu.{}` is a channel, not a register", member.value),
                format!(
                    "write one of its registers, such as `apu.{}.{}`",
                    member.value,
                    registers(&member.value).map_or("", |registers| registers[0])
                ),
            ),
            Direct::Dmc => self.not_in_this_release_with(whole.span, DMC_MESSAGE, DMC_LABEL),
            Direct::BelongsTo(channel) => self.error_with(
                whole.span,
                format!("unknown apu register `{}`", member.value),
                format!(
                    "`{0}` belongs to a channel, such as `apu.{channel}.{0}`",
                    member.value
                ),
            ),
            Direct::Unknown => self.error_with(
                whole.span,
                format!("unknown apu register `{}`", member.value),
                APU_LIST,
            ),
        }
        ValueType::Unknown
    }

    /// `apu.<channel>.<register>`.
    pub(super) fn apu_channel_type(
        &mut self,
        whole: &Spanned<Expression>,
        channel: &Spanned<String>,
        member: &Spanned<String>,
    ) -> ValueType {
        if channel.value == "dmc" {
            self.not_in_this_release_with(whole.span, DMC_MESSAGE, DMC_LABEL);
            return ValueType::Unknown;
        }
        let Some(known) = registers(&channel.value) else {
            self.error_with(
                whole.span,
                format!("unknown apu channel `{}`", channel.value),
                CHANNEL_LIST,
            );
            return ValueType::Unknown;
        };
        if known.contains(&member.value.as_str()) {
            return ValueType::U8;
        }
        self.error_with(
            whole.span,
            format!("unknown apu.{} register `{}`", channel.value, member.value),
            format!("{} has {}", channel.value, english_list(known)),
        );
        ValueType::Unknown
    }
}
