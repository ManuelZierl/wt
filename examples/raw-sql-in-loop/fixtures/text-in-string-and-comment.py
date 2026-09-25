def sync_users(user_ids):
    # SELECT * FROM users WHERE id = %s -- do not do this inside the loop
    note = "SELECT * FROM users WHERE id = %s"
    for user_id in user_ids:
        log(note, user_id)
