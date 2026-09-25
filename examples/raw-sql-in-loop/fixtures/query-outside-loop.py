def sync_users(user_ids):
    cursor.execute("SELECT * FROM users WHERE id IN %s", [tuple(user_ids)])
    rows = cursor.fetchall()
    for row in rows:
        process(row)
