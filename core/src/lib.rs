//! Núcleo compartido de OpenCam.
//!
//! Contiene el protocolo OPCM v1, la cola de frames y, en fases posteriores,
//! métricas, reconexión y descubrimiento. Se expone a Android/iOS/escritorio por FFI.

pub mod frame_queue;
pub mod protocol;
pub mod shared_frame;

pub use frame_queue::{Frame, FrameQueue};
pub use protocol::{Header, MsgType, ProtoError, DEFAULT_PORT, FLAG_KEYFRAME, HEADER_LEN};
