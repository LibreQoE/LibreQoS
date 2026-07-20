export function websocketHelloReply(ackText) {
    return {
        HelloReply: {
            ack: ackText,
            token: "",
        },
    };
}
