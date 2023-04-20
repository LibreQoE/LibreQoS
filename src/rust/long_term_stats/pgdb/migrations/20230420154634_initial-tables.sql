-- Creates the initial tables for the license server
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
    last_seen timestamp without time zone DEFAULT now() NOT NULL,
    public_key bytea
);

CREATE TABLE public.stats_hosts (
    id integer NOT NULL,
    ip_address character varying(128) NOT NULL,
    can_accept_new_clients boolean NOT NULL DEFAULT true,
    influx_host character varying(128) NOT NULL
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

