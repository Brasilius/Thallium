-- ============================================================================
-- Thallium Initial Schema (Phase 2: Objective 5)
-- Postgres secondary index for fast sorting and querying of Minecraft
-- player inventory snapshots stored in S3/R2-style object storage.
-- ============================================================================

-- 1. Players
-- Tracks unique Minecraft players seen in playerdata snapshots.
CREATE TABLE IF NOT EXISTS players (
    uuid         UUID        PRIMARY KEY,
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 2. Snapshots
-- Links each captured player inventory snapshot to its object storage location.
CREATE TABLE IF NOT EXISTS snapshots (
    id          BIGSERIAL   PRIMARY KEY,
    player_uuid UUID        NOT NULL REFERENCES players(uuid) ON DELETE CASCADE,
    bucket      TEXT        NOT NULL,
    object_key  TEXT        NOT NULL,
    etag        TEXT,
    captured_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT uq_snapshots_bucket_object_key UNIQUE (bucket, object_key)
);

-- 3. Inventory Items
-- Flattened item entries extracted from a player's Inventory and EnderItems,
-- including recursive nested items inside shulker boxes and bundles via
-- `parent_item_id`.
CREATE TABLE IF NOT EXISTS inventory_items (
    id             BIGSERIAL PRIMARY KEY,
    snapshot_id    BIGINT    NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
    parent_item_id BIGINT    REFERENCES inventory_items(id) ON DELETE CASCADE,
    slot           SMALLINT, -- Nullable for unslotted containers (e.g., bundles)
    slot_range     TEXT      NOT NULL CHECK (
        slot_range IN (
            'hotbar',
            'main',
            'armor',
            'offhand',
            'ender_chest',
            'shulker_box',
            'bundle'
        )
    ),
    item_id        TEXT      NOT NULL,
    count          INTEGER   NOT NULL CHECK (count > 0)
);

-- 4. Item Components
-- Modern Minecraft data components (or translated legacy NBT tag metadata)
-- attached to an inventory item.
CREATE TABLE IF NOT EXISTS item_components (
    id                BIGSERIAL PRIMARY KEY,
    inventory_item_id BIGINT    NOT NULL REFERENCES inventory_items(id) ON DELETE CASCADE,
    component_key     TEXT      NOT NULL,
    component_value   JSONB     NOT NULL,

    CONSTRAINT uq_item_components_item_key UNIQUE (inventory_item_id, component_key)
);

-- ============================================================================
-- Indexes
-- Optimized for filtering, sorting, container traversal, and FK joins/cascades.
-- ============================================================================

-- Snapshots: filter by player, sort by capture time, check etag deduplication
CREATE INDEX IF NOT EXISTS idx_snapshots_player_uuid_captured_at
    ON snapshots (player_uuid, captured_at DESC);

CREATE INDEX IF NOT EXISTS idx_snapshots_captured_at
    ON snapshots (captured_at DESC);

CREATE INDEX IF NOT EXISTS idx_snapshots_player_uuid_etag
    ON snapshots (player_uuid, etag)
    WHERE etag IS NOT NULL;

-- Inventory items: FK joins, nested container traversal, and filter/sort columns
CREATE INDEX IF NOT EXISTS idx_inventory_items_snapshot_id
    ON inventory_items (snapshot_id);

CREATE INDEX IF NOT EXISTS idx_inventory_items_parent_item_id
    ON inventory_items (parent_item_id)
    WHERE parent_item_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_inventory_items_item_id
    ON inventory_items (item_id);

CREATE INDEX IF NOT EXISTS idx_inventory_items_slot_range
    ON inventory_items (slot_range);

CREATE INDEX IF NOT EXISTS idx_inventory_items_count
    ON inventory_items (count DESC);

-- Item components: FK join + component key/value lookups
CREATE INDEX IF NOT EXISTS idx_item_components_inventory_item_id
    ON item_components (inventory_item_id);

CREATE INDEX IF NOT EXISTS idx_item_components_component_key
    ON item_components (component_key);

CREATE INDEX IF NOT EXISTS idx_item_components_value_gin
    ON item_components USING GIN (component_value);
