//! Each built-in device's `model.rs`, built in a crate whose only dependency is `orbe-model`. A
//! device that names anything outside `orbe_model` and `std` does not build here, so the
//! workspace build and its tests fail. Nothing calls the devices, so unused code is allowed.
#![allow(dead_code)]

include!(concat!(env!("OUT_DIR"), "/devices.rs"));
