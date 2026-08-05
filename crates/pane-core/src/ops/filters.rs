use uuid::Uuid;

use pane_ipc::{kinds, FilterDto, SaveFilterArgs};

use crate::error::{to_api, CoreResult};
use crate::Core;

impl Core {
    pub async fn filter_save(&self, args: SaveFilterArgs) -> CoreResult<FilterDto> {
        self.storage.save_filter(args).map_err(to_api(kinds::DB))
    }

    /// `kind` is `"captures"` or `"logcat"`; `None` returns both.
    pub async fn filters_list(&self, kind: Option<&str>) -> CoreResult<Vec<FilterDto>> {
        self.storage.list_filters(kind).map_err(to_api(kinds::DB))
    }

    pub async fn filter_delete(&self, id: Uuid) -> CoreResult<()> {
        self.storage.delete_filter(id).map_err(to_api(kinds::DB))
    }
}
