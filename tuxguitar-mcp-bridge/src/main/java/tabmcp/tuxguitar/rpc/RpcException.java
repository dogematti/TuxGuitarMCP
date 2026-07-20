package tabmcp.tuxguitar.rpc;

/** An error to be returned to the client as a JSON-RPC error response. */
public class RpcException extends Exception {

	private static final long serialVersionUID = 1L;

	public static final String NOT_AUTHENTICATED = "E_NOT_AUTHENTICATED";
	public static final String PROTOCOL_VERSION = "E_PROTOCOL_VERSION";
	public static final String NO_DOCUMENT = "E_NO_DOCUMENT";
	public static final String INVALID_RANGE = "E_INVALID_RANGE";
	public static final String STALE_REVISION = "E_STALE_REVISION";
	public static final String EDIT_FAILED = "E_EDIT_FAILED";
	public static final String LOCKED = "E_LOCKED";
	public static final String UNSUPPORTED = "E_UNSUPPORTED";
	public static final String INTERNAL = "E_INTERNAL";

	private final String code;

	public RpcException(String code, String message) {
		super(message);
		this.code = code;
	}

	public String getCode() {
		return this.code;
	}
}
