-- What an account keeps for itself in the panel: the language and the menu entries it pinned.
-- No row means the defaults (no language chosen, nothing pinned).
CREATE TABLE account_preferences (
    account_id uuid        PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    locale     text
        CONSTRAINT account_preferences_locale CHECK (locale IN ('en', 'bn')),
    favorites  text[]      NOT NULL DEFAULT '{}'
        CONSTRAINT account_preferences_favorites_count CHECK (cardinality(favorites) <= 30),
    updated_at timestamptz NOT NULL DEFAULT now()
);
