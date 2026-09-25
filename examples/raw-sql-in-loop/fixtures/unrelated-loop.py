def sync_users(user_ids):
    for user_id in user_ids:
        log(user_id)
        notify(user_id)
