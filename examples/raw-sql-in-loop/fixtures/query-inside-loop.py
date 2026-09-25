def sync_users(user_ids):
    for user_id in user_ids:
        cursor.execute("SELECT * FROM users WHERE id = %s", [user_id])
        row = cursor.fetchone()
        process(row)
