def process(users):
    for user in users:
        # Profile.objects.filter(user=user) would be an N+1 query
        note = "Profile.objects.filter(user=user)"
        send(user, note)
