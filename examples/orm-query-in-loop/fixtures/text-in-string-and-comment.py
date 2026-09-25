def process(users):
    for user in users:
        # user.objects.filter(id=1) would be an N+1 query
        note = "user.objects.filter(id=1)"
        send(user, note)
