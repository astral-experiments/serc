async def probe(request, close_sessions, made_sessions):
    token = ''
    url = ''
    try:
        response = await request()
        if response is None:
            return Output(output_dict={})
        else:
            token = response.get('token')
            url = response.get('url')
    except Exception as error:
        return Output(output_dict={'success': False, 'error': str(error)})
    finally:
        if made_sessions:
            await close_sessions()
    return token, url
