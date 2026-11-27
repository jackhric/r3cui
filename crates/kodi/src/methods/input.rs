//! `Input.*`: Kodi's own navigation keys, and named actions for the rest.

use crate::methods::simple;
use crate::rpc::Method;
use serde::Serialize;

simple!(Up = "Input.Up");
simple!(Down = "Input.Down");
simple!(Left = "Input.Left");
simple!(Right = "Input.Right");
simple!(Select = "Input.Select");
simple!(Back = "Input.Back");
simple!(Home = "Input.Home");
simple!(ContextMenu = "Input.ContextMenu");
simple!(Info = "Input.Info");
simple!(ShowOsd = "Input.ShowOSD");

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ExecuteAction {
    pub action: InputAction,
}
impl Method for ExecuteAction {
    const NAME: &'static str = "Input.ExecuteAction";
    type Output = serde::de::IgnoredAny;
}

/// The subset of the schema's 215 `Input.Action` names the remote sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InputAction {
    PageUp,
    PageDown,
    SkipNext,
    SkipPrevious,
    PlayPause,
    Stop,
    Osd,
    ChapterOrBigStepForward,
    ChapterOrBigStepBack,
}
