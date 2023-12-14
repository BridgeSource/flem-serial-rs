# FLEM Serial Rust
This library is meant to be an easy to use serial implementation of FLEM. 

## Changelog

### 0.3.1
- Expanded `ConnectionSettings` to include parity, stop bits, and flow control.
- Added `update_connection_settings` to `FlemSerial` to alter the connection settings.

### 0.3.0
- Updated `flem-serial-rs` to use the new `flem::traits::Channel`, this is a breaking change

## What is FLEM
[FLEM stands for Flexible, Lightweight, Embedded Messaging](https://github.com/BridgeSource/flem-rs) protocol. It is
designed to work on embedded targets as well as host devices using a common codebase to encode and decode any type 
of data that can be converted to a byte array.

## Why would I use this?
If you need to get data to and from an embedded target using UART communications, this is a candidate solution.

## How do I use this?
- Setup FLEM on an embedded target. This is extremely device specific, but we have used FLEM on the TM4C and STM32 processors.
Stay tuned for a starter project on the STM32 Nucleo. In general, configure the embedded targets UART peripheral, and follow
examples in FLEM to `construct()` a packet, handle the packet, and transmit a response. Alternatively, the embedded target
can spontanesouly emit packet through events.
- Setup a Rust program on the host device that uses this library. Follow the `flem_serial_example.rs` to list ports, connect 
to a target port, send packets, and receive / handle packets. 