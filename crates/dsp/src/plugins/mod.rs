pub mod drums;
pub mod dynamics;
pub mod eq;
pub mod harmonic;
pub mod linmb_table;
pub mod modulation;
pub mod open_comp;
pub mod open_drt;
pub mod open_echo;
pub mod open_eq;
pub mod open_g;
pub mod open_mb;
pub mod open_r;
pub mod open_sat;
pub mod pitch;
pub mod reverb;
pub mod sampler;
pub mod strip;
pub mod synth;
pub mod utility;

use crate::{Plugin, PluginInfo};

pub(crate) static REGISTRY: &[&PluginInfo] = &[
    &open_eq::OPEN_EQ_INFO,
    &open_comp::OPEN_COMP_INFO,
    &open_g::OPEN_G_INFO,
    &open_mb::OPEN_MB_INFO,
    &open_sat::OPEN_SAT_INFO,
    &open_drt::OPEN_DRT_INFO,
    &open_r::OPEN_R_INFO,
    &open_echo::OPEN_ECHO_INFO,
    &eq::EQ1_INFO,
    &eq::EQ7_INFO,
    &dynamics::COMP_INFO,
    &dynamics::GATE_INFO,
    &dynamics::DEESS_INFO,
    &dynamics::MAX_INFO,
    &strip::STRIP_INFO,
    &reverb::ROOM_INFO,
    &reverb::PLATE_INFO,
    &modulation::DELAY_INFO,
    &modulation::CHORUS_INFO,
    &modulation::FLANGER_INFO,
    &modulation::PHASER_INFO,
    &harmonic::SAT_INFO,
    &harmonic::LOFI_INFO,
    &harmonic::RECT_INFO,
    &pitch::PITCH_INFO,
    &utility::SHIFT_INFO,
    &utility::GAIN_INFO,
    &utility::TRIM_INFO,
    &utility::INVERT_INFO,
    &utility::DC_INFO,
    &utility::GEN_INFO,
    &utility::DITHER_INFO,
    &synth::SYNTH_INFO,
    &drums::DRUM_INFO,
    &sampler::SAMPLER_INFO,
];

pub(crate) const RETIRED: &[&str] = &[
    "eq_1band",
    "eq_7band",
    "compressor",
    "expander_gate",
    "de_esser",
    "maximizer",
    "channel_strip",
    "room_reverb",
    "plate_reverb",
    "mod_delay",
    "chorus",
    "flanger",
    "phaser",
    "saturator",
    "lofi",
    "rectifier",
    "pitch_shifter",
    "dither",
];

pub(crate) fn instantiate(id: &str) -> Option<Box<dyn Plugin>> {
    Some(match id {
        "eq_1band" => Box::new(eq::Eq1::new()),
        "eq_7band" => Box::new(eq::Eq7::new()),
        "compressor" => Box::new(dynamics::Compressor::new()),
        "expander_gate" => Box::new(dynamics::ExpanderGate::new()),
        "de_esser" => Box::new(dynamics::DeEsser::new()),
        "maximizer" => Box::new(dynamics::Maximizer::new()),
        "channel_strip" => Box::new(strip::ChannelStrip::new()),
        "room_reverb" => Box::new(reverb::Reverb::room()),
        "plate_reverb" => Box::new(reverb::Reverb::plate()),
        "mod_delay" => Box::new(modulation::ModDelay::new()),
        "chorus" => Box::new(modulation::ModFx::chorus()),
        "flanger" => Box::new(modulation::ModFx::flanger()),
        "phaser" => Box::new(modulation::Phaser::new()),
        "saturator" => Box::new(harmonic::Saturator::new()),
        "lofi" => Box::new(harmonic::LoFi::new()),
        "rectifier" => Box::new(harmonic::Rectifier::new()),
        "pitch_shifter" => Box::new(pitch::PitchShifter::new()),
        "time_shift" => Box::new(utility::TimeShift::new()),
        "gain" => Box::new(utility::Gain::new()),
        "trim" => Box::new(utility::Trim::new()),
        "invert" => Box::new(utility::Invert::default()),
        "dc_offset_removal" => Box::new(utility::DcRemoval::new()),
        "signal_generator" => Box::new(utility::SignalGenerator::new()),
        "dither" => Box::new(utility::Dither::new()),
        "subtractive_synth" => Box::new(synth::SubtractiveSynth::new()),
        "drum_synth" => Box::new(drums::DrumSynth::new()),
        "sampler" => Box::new(sampler::Sampler::new()),
        "open_eq" => Box::new(open_eq::OpenEq::new()),
        "open_mb" => Box::new(open_mb::OpenMb::new()),
        "open_sat" => Box::new(open_sat::OpenSat::new()),
        "open_comp" => Box::new(open_comp::OpenComp::new()),
        "open_drt" => Box::new(open_drt::OpenDrt::new()),
        "open_g" => Box::new(open_g::OpenG::new()),
        "open_r" => Box::new(open_r::OpenR::new()),
        "open_echo" => Box::new(open_echo::OpenEcho::new()),
        _ => return None,
    })
}
