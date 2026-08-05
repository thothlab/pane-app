use pane_ipc::{kinds, ReplayRecordDto, ReplaySendArgs};

use crate::error::{to_api, CoreResult};
use crate::Core;

impl Core {
    pub async fn replay_send(&self, args: ReplaySendArgs) -> CoreResult<ReplayRecordDto> {
        self.storage
            .replay_send(args)
            .await
            .map_err(to_api(kinds::REPLAY_FAILED))
    }
}
