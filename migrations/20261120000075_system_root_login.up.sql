-- Logging in to the panel as the server's `root`, with the server's own root password (like
-- WHM). The administrator account named `root` keeps no password of its own: its password_hash
-- is the marker '!system:root', and the password is checked against /etc/shadow by the small
-- root helper (builtpanel-root-auth). Only that one account may carry the marker.
ALTER TABLE accounts DROP CONSTRAINT accounts_password_hash_phc;
ALTER TABLE accounts ADD CONSTRAINT accounts_password_hash_phc CHECK (
    password_hash LIKE '$argon2id$%'
    OR (password_hash = '!system:root' AND username = 'root' AND kind = 'admin')
);
