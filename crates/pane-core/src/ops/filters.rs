use uuid::Uuid;

use pane_ipc::{kinds, FilterDto, SaveFilterArgs};

use crate::error::{to_api, CoreResult};
use crate::Core;

impl Core {
    pub async fn filter_save(&self, args: SaveFilterArgs) -> CoreResult<FilterDto> {
        self.db(move |s| s.save_filter(args))
            .await
            .map_err(to_api(kinds::DB))
    }

    /// `kind` is `"captures"` or `"logcat"`; `None` returns both.
    pub async fn filters_list(&self, kind: Option<&str>) -> CoreResult<Vec<FilterDto>> {
        let kind = kind.map(str::to_string);
        self.db(move |s| s.list_filters(kind.as_deref()))
            .await
            .map_err(to_api(kinds::DB))
    }

    pub async fn filter_delete(&self, id: Uuid) -> CoreResult<()> {
        self.db(move |s| s.delete_filter(id))
            .await
            .map_err(to_api(kinds::DB))
    }
}
