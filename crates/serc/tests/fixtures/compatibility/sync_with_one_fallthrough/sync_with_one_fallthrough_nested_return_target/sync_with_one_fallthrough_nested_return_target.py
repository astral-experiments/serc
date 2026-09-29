def check_device(direct, bridge, credentials, cluster, search, logger):
    if direct:
        return False
    else:
        with bridge(
            credentials=credentials, cluster=cluster
        ) as connection:
            output, status = connection.exec_command('show model')
            match = search('component=(.+)', output)
            if not match:
                match = search('model=(.+)', output)
            if match:
                value = match.group(1).strip()
                if value == 'SUPPORTED':
                    logger.debug('Supported model')
                    return True
                else:
                    logger.error('Unknown model: %s', value)
                    return False
            else:
                logger.error('Model missing: %s', output)
        logger.error('Unable to confirm model')
        return False
