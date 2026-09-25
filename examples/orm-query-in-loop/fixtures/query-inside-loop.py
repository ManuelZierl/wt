def process(request):
    users = User.objects.all()
    for user in users:
        profile = Profile.objects.filter(owner=user)
        send(profile)
