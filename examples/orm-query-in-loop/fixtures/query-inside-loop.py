def process(request):
    users = User.objects.all()
    for user in users:
        profile = user.objects.filter(owner=user)
        send(profile)
