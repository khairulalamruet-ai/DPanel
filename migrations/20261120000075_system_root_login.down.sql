-- The account that used the server's root password can no longer log in without it.
UPDATE accounts SET status = 'disabled', password_hash = '$argon2id$v=19$m=19456,t=2,p=1$disabled$disabled'
WHERE password_hash = '!system:root';
ALTER TABLE accounts DROP CONSTRAINT accounts_password_hash_phc;
ALTER TABLE accounts ADD CONSTRAINT accounts_password_hash_phc CHECK (password_hash LIKE '$argon2id$%');
