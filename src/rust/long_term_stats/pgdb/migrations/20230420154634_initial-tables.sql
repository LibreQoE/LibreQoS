-- Creates the initial tables for the license server

-- We're using Trigrams for faster text search
CREATE EXTENSION pg_trgm;

CREATE TABLE public.licenses (
    key character varying(254) NOT NULL,
    stats_host integer NOT NULL
);

CREATE TABLE public.organizations (
    key character varying(254) NOT NULL,
    name character varying(254) NOT NULL,
    influx_host character varying(254) NOT NULL,
    influx_org character varying(254) NOT NULL,
    influx_token character varying(254) NOT NULL,
    influx_bucket character varying(254) NOT NULL
);

CREATE TABLE public.shaper_nodes (
    license_key character varying(254) NOT NULL,
    node_id character varying(254) NOT NULL,
    node_name character varying(254) NOT NULL,
    last_seen timestamp without time zone DEFAULT now() NOT NULL,
    public_key bytea
);

CREATE TABLE public.site_tree
(
    key character varying(254) NOT NULL,
    site_name character varying(254) NOT NULL,
    host_id character varying(254) NOT NULL,
    index integer NOT NULL,
    parent integer NOT NULL,
    site_type character varying(32),
    max_up integer NOT NULL DEFAULT 0,
    max_down integer NOT NULL DEFAULT 0,
    current_up integer NOT NULL DEFAULT 0,
    current_down integer NOT NULL DEFAULT 0,
    current_rtt integer NOT NULL DEFAULT 0,
    PRIMARY KEY (key, site_name, host_id)
);

CREATE TABLE public.shaped_devices
(
    key character varying(254) NOT NULL,
    node_id character varying(254) NOT NULL,
    circuit_id character varying(254) NOT NULL,
    device_id character varying(254) NOT NULL,
    circuit_name character varying(254) NOT NULL DEFAULT '',
    device_name character varying(254) NOT NULL DEFAULT '',
    parent_node character varying(254) NOT NULL DEFAULT '',
    mac character varying(254) NOT NULL DEFAULT '',
    download_min_mbps integer NOT NULL DEFAULT 0,
    upload_min_mbps integer NOT NULL DEFAULT 0,
    download_max_mbps integer NOT NULL DEFAULT 0,
    upload_max_mbps integer NOT NULL DEFAULT 0,
    comment text,
    PRIMARY KEY (key, node_id, circuit_id, device_id)
);

CREATE TABLE public.shaped_device_ip
(
    key character varying(254) COLLATE pg_catalog."default" NOT NULL,
    node_id character varying(254) COLLATE pg_catalog."default" NOT NULL,
    circuit_id character varying(254) COLLATE pg_catalog."default" NOT NULL,
    ip_range character varying(254) COLLATE pg_catalog."default" NOT NULL,
    subnet integer NOT NULL,
    CONSTRAINT shaped_device_ip_pkey PRIMARY KEY (key, node_id, circuit_id, ip_range, subnet)
);

CREATE TABLE public.stats_hosts (
    id integer NOT NULL,
    ip_address character varying(128) NOT NULL,
    can_accept_new_clients boolean NOT NULL DEFAULT true,
    influx_host character varying(128) NOT NULL,
    api_key character varying(255) NOT NULL
);

CREATE SEQUENCE public.stats_hosts_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;

ALTER TABLE ONLY public.stats_hosts 
    ALTER COLUMN id SET DEFAULT nextval('public.stats_hosts_id_seq'::regclass);

ALTER TABLE ONLY public.licenses
    ADD CONSTRAINT licenses_pkey PRIMARY KEY (key);

ALTER TABLE ONLY public.organizations
    ADD CONSTRAINT pk_organizations PRIMARY KEY (key);

ALTER TABLE ONLY public.shaper_nodes
    ADD CONSTRAINT shaper_nodes_pk PRIMARY KEY (license_key, node_id);

ALTER TABLE ONLY public.stats_hosts
    ADD CONSTRAINT stats_hosts_pkey PRIMARY KEY (id);

ALTER TABLE ONLY public.organizations
    ADD CONSTRAINT organizations_license_fk FOREIGN KEY (key) REFERENCES public.licenses(key);

ALTER TABLE ONLY public.licenses
    ADD CONSTRAINT stats_host_fk FOREIGN KEY (stats_host) REFERENCES public.stats_hosts(id) NOT VALID;

CREATE TABLE public.logins
(
    key character varying(254) NOT NULL,
    username character varying(64) NOT NULL,
    password_hash character varying(64) NOT NULL,
    nicename character varying(64) NOT NULL,
    CONSTRAINT pk_logins_licenses PRIMARY KEY (key, username),
    CONSTRAINT fk_login_licenses FOREIGN KEY (key)
        REFERENCES public.licenses (key) MATCH SIMPLE
        ON UPDATE NO ACTION
        ON DELETE NO ACTION
        NOT VALID
);

CREATE TABLE public.active_tokens
(
    key character varying(254) NOT NULL,
    token character varying(254) NOT NULL,
    username character varying(64) NOT NULL,
    expires timestamp without time zone NOT NULL DEFAULT NOW() + interval '2 hours',
    PRIMARY KEY (token)
);

CREATE TABLE public.uisp_devices_ext
(
    key character varying(254) NOT NULL,
    device_id character varying(254) NOT NULL,
    name character varying(254) NOT NULL DEFAULT '',
    model character varying(254) NOT NULL DEFAULT '',
    firmware character varying(64) NOT NULL DEFAULT '',
    status character varying(64) NOT NULL DEFAULT '',
    mode character varying(64) NOT NULL DEFAULT '',
    channel_width integer NOT NULL DEFAULT 0,
    tx_power integer NOT NULL DEFAULT 0,
    PRIMARY KEY (key, device_id)
);

CREATE TABLE public.uisp_devices_interfaces
(
    key character varying(254) NOT NULL,
    device_id character varying(254) NOT NULL,
    id serial NOT NULL,
    name character varying(64) NOT NULL DEFAULT '',
    mac character varying(64) NOT NULL DEFAULT '',
    status character varying(64) NOT NULL DEFAULT '',
    speed character varying(64) NOT NULL DEFAULT '',
    ip_list character varying(254) NOT NULL DEFAULT '',
    PRIMARY KEY (key, device_id, id)
);

---- Indices

CREATE INDEX site_tree_key
    ON public.site_tree USING btree
    (key ASC NULLS LAST)
;

CREATE INDEX site_tree_key_parent
    ON public.site_tree USING btree
    (key ASC NULLS LAST, parent ASC NULLS LAST)
;

CREATE INDEX shaped_devices_key_circuit_id
    ON public.shaped_devices USING btree
    (key ASC NULLS LAST, circuit_id ASC NULLS LAST)
;

CREATE INDEX stats_host_ip
    ON public.stats_hosts USING btree
    (ip_address ASC NULLS LAST)
;

CREATE INDEX shaper_nodes_license_key_idx
    ON public.shaper_nodes USING btree
    (license_key ASC NULLS LAST)
;