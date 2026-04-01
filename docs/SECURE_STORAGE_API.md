# Secure Storage API Documentation

This document describes the Tauri commands available for the secure local storage and NIP-78 encrypted relay backup system.

## Overview

The secure storage system provides:
- **Local encrypted storage** of Nostr accounts using AES-256-GCM
- **Fast login** (~4 seconds) without NIP-46 reconnection
- **NIP-78 encrypted relay backup** for cross-device synchronization
- **Account management** with multiple account support

## Account Manager Commands

### `login_with_nsec`
Creates a new encrypted local account from an nsec private key.

**Parameters:**
- `nsec` (String): The nsec private key (nsec1... or hex format)
- `name` (Option<String>): Optional display name for the account

**Returns:**
```json
{
  "success": true,
  "account": {
    "id": "user_01H...",
    "npub": "npub1...",
    "name": "My Account",
    "signing_mode": "Local",
    "last_used": 1234567890
  }
}
```

**Example:**
```javascript
const result = await invoke('login_with_nsec', {
  nsec: 'nsec1...',
  name: 'My Gaming Account'
});
```

---

### `load_active_account`
Loads the currently active account for fast login. This is the primary method for app startup.

**Parameters:** None

**Returns:**
```json
{
  "success": true,
  "account": {
    "id": "user_01H...",
    "npub": "npub1...",
    "name": "My Account",
    "signing_mode": "Local",
    "last_used": 1234567890
  }
}
```

**Error:** Returns error if no active account exists.

**Example:**
```javascript
try {
  const result = await invoke('load_active_account');
  // User is logged in, proceed to app
} catch (e) {
  // No active account, show login screen
}
```

---

### `list_accounts`
Lists all stored accounts for account switching UI.

**Parameters:** None

**Returns:**
```json
{
  "success": true,
  "accounts": [
    {
      "id": "user_01H...",
      "npub": "npub1...",
      "name": "Account 1",
      "signing_mode": "Local",
      "last_used": 1234567890
    },
    {
      "id": "user_01H...",
      "npub": "npub1...",
      "name": "Account 2",
      "signing_mode": "Remote",
      "last_used": 1234567890
    }
  ]
}
```

**Example:**
```javascript
const { accounts } = await invoke('list_accounts');
// Display account list for switching
```

---

### `switch_account`
Switches to a different account and loads it.

**Parameters:**
- `account_id` (String): The account ID to switch to

**Returns:**
```json
{
  "success": true,
  "account": {
    "id": "user_01H...",
    "npub": "npub1...",
    "name": "Switched Account",
    "signing_mode": "Local",
    "last_used": 1234567890
  }
}
```

**Example:**
```javascript
await invoke('switch_account', { account_id: 'user_01H...' });
// Account is now active and ready for use
```

---

### `delete_account`
Deletes an account from local storage.

**Parameters:**
- `account_id` (String): The account ID to delete

**Returns:**
```json
{
  "success": true,
  "message": "Account deleted successfully"
}
```

**Example:**
```javascript
await invoke('delete_account', { account_id: 'user_01H...' });
```

---

### `has_accounts`
Checks if any accounts exist (useful for migration checks).

**Parameters:** None

**Returns:** `boolean`

**Example:**
```javascript
const hasAccounts = await invoke('has_accounts');
if (!hasAccounts) {
  // Show first-time setup or migration prompt
}
```

---

## NIP-78 Backup Commands

### `create_backup`
Creates an encrypted backup of all accounts.

**Parameters:** None

**Returns:**
```json
{
  "success": true,
  "backup": "{\"version\":1,\"encrypted_accounts\":\"...\"}",
  "message": "Backup created successfully"
}
```

**Security Notes:**
- Backup is encrypted with the same master key as local storage
- The backup string can be stored safely (it's already encrypted)
- Master key never leaves the device

**Example:**
```javascript
const { backup } = await invoke('create_backup');
// Store backup string securely or display to user for manual backup
```

---

### `restore_backup`
Restores accounts from an encrypted backup.

**Parameters:**
- `backup_data` (String): The encrypted backup JSON string

**Returns:**
```json
{
  "success": true,
  "restored_count": 2,
  "message": "Restored 2 accounts"
}
```

**Notes:**
- Duplicate accounts (same npub) are skipped
- Restored accounts are not auto-activated
- Use `switch_account` to activate a restored account

**Example:**
```javascript
const result = await invoke('restore_backup', {
  backup_data: backupString
});
console.log(`Restored ${result.restored_count} accounts`);
```

---

### `publish_backup_to_relays`
Creates and publishes an encrypted backup to Nostr relays.

**Parameters:** None

**Returns:**
```json
{
  "success": true,
  "message": "Backup created successfully (relay publishing pending implementation)",
  "backup_size": 1234,
  "public_key": "npub1..."
}
```

**Note:** Full relay publishing integration is pending. Currently creates the backup and returns metadata.

**Example:**
```javascript
const result = await invoke('publish_backup_to_relays');
// Backup is ready for publishing to relays
```

---

## Legacy Commands (Still Supported)

The following legacy commands continue to work alongside the new system:

- `connect_with_key` - Direct key login (creates temporary session, not stored)
- `connect_nip46` - NIP-46 remote signer connection
- `generate_nostrconnect_uri` - Generate NostrConnect URI
- `wait_for_nostrconnect_signer` - Wait for NostrConnect handshake
- `get_public_key` - Get current user's public key
- `is_authenticated` - Check authentication status
- `disconnect` - Clear authentication state

---

## Migration from Legacy System

The app automatically migrates from the old `saved_users.json` system:

1. On first run with new system, check `has_accounts()`
2. If false, check for legacy saved users
3. Migrate legacy users to new encrypted database
4. Legacy file is preserved as backup

**Migration behavior:**
- `DirectKey` → `Local` signing mode (encrypted nsec storage)
- `Nostrconnect`/`Bunker` → `Remote` signing mode (NIP-46)

---

## Security Considerations

### Encryption
- **Algorithm:** AES-256-GCM with authenticated encryption
- **Master Key:** 256-bit key stored in secure file (0600 permissions)
- **nsec Storage:** Encrypted individually, decrypted only when signing
- **Memory Safety:** Uses `Zeroizing` to clear sensitive data from memory

### Backup Security
- **Double Encryption:** Accounts encrypted with master key, backup container encrypted again
- **Relay Safety:** Only encrypted data is published to relays
- **No Key Exposure:** Master key never leaves the device

### Best Practices
1. Always use `load_active_account()` on app startup for fast login
2. Call `switch_account()` when user selects different account
3. Create backups regularly (especially after adding new accounts)
4. Store backup strings securely (password manager, encrypted file, etc.)

---

## Error Handling

All commands return `Result<T, String>` where errors are descriptive strings:

```javascript
try {
  const result = await invoke('login_with_nsec', { nsec: 'invalid' });
} catch (error) {
  // error is a string describing what went wrong
  console.error('Login failed:', error);
}
```

Common errors:
- `"Invalid nsec format"` - Malformed private key
- `"Account not found"` - Invalid account ID
- `"Not authenticated"` - No active session
- `"Encryption failed"` - Cryptographic error
- `"Database error"` - SQLite operation failed

---

## Implementation Status

✅ **Completed:**
- Local encrypted account storage
- Fast login from database
- Account management (CRUD operations)
- NIP-78 backup creation and restore
- Tauri command integration

🔄 **Pending:**
- Full relay publishing integration (requires NostrClient event signing)
- Automatic backup on account changes
- Backup scheduling/periodic backups
- Cross-device restore UI

---

## Frontend Integration Example

```javascript
// App startup
async function initializeApp() {
  try {
    // Try fast login from local database
    const result = await invoke('load_active_account');
    console.log('Fast login successful:', result.account.npub);
    return { authenticated: true, account: result.account };
  } catch (e) {
    // Check if we have any accounts
    const hasAccounts = await invoke('has_accounts');
    if (hasAccounts) {
      // Show account selection screen
      const { accounts } = await invoke('list_accounts');
      return { authenticated: false, accounts };
    } else {
      // Show login screen
      return { authenticated: false, accounts: [] };
    }
  }
}

// Login with nsec
async function loginWithNsec(nsec, name) {
  const result = await invoke('login_with_nsec', { nsec, name });
  return result.account;
}

// Switch account
async function switchAccount(accountId) {
  const result = await invoke('switch_account', { account_id: accountId });
  return result.account;
}

// Create backup
async function createBackup() {
  const result = await invoke('create_backup');
  return result.backup; // Encrypted backup string
}

// Restore from backup
async function restoreFromBackup(backupString) {
  const result = await invoke('restore_backup', { backup_data: backupString });
  return result.restored_count;
}
```

---

## Support

For issues or questions about the secure storage system:
1. Check the logs for detailed error messages
2. Verify the data directory permissions
3. Ensure the master key file exists and is readable
4. Review the backup/restore flow for any data inconsistencies
