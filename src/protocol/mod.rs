//! The messages of the input method servers that keep a Chinese/English mode of their own: Doubao's
//! `ImeService` and Mingkuai's `mingkuaid`. These modules only build and parse messages. The pipe
//! and socket I/O lives in `win` and `mac`.

pub mod doubao;
pub mod mingkuai;
